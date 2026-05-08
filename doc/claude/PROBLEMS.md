
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
| 201 | `tests/html_wasm.rs` Mutex-poison cascade: when one test panicked inside `build_lock().lock().unwrap()` (line 174), every subsequent html_wasm test failed with the unhelpful `called Result::unwrap() on an Err value: PoisonError { .. }` instead of the original error message.  **Closed (2026-05-06)**: the lock acquire now uses `.unwrap_or_else(\|e\| e.into_inner())` to consume a poisoned guard rather than re-panic.  The lock guards a shared file path (`/tmp/loft_html.rs`), not invariant state — the next test's `loft --html` invocation overwrites whatever the panicking test left half-written, so consuming the poisoned guard is safe.  `assert_wasm_rlib_fresh()` already runs before the lock acquire so a stale rlib fails the test before it can poison the build serial.  Pinned by `tests/html_wasm.rs::p201_poisoned_lock_recovery_pattern` — exercises the recovery shape on a local `Mutex<()>` so a regression (someone reverts to `.unwrap()`) trips here without relying on cascading real failures to demonstrate the bug. | Low (closed) | n/a — fix lands in `tests/html_wasm.rs::run_html_wasm_with_libs`. |
| 203 | Assertion `delete(path) == FileResult.Ok` panicked with `delete existing file`.  **Closed (2026-05-02)**: actual root cause was template double-substitution — five templates in `default/01_code.loft` (lines 690, 705, 707, 751, 753) substituted `@v1`/`@v2` in multiple positions, so any side-effecting call appearing on both sides of an enum/null comparison evaluated twice.  Fix: `src/generation/calls.rs::output_call_template` now scans every template for repeated `@<name>` placeholders and hoists each into a single `let _v_<name> = …;` wrapping `{ … }` block before substitution, so each arg expression evaluates exactly once regardless of how many positions reference it.  `repro_p203.loft` exits 0; full suite: 540/540 issues, 43/43 threading, 35/35 threading_chars, native 86/92 → 87/92.  See [P203 reproducer + diagnosis](#203-native-block-scope-file-not-flushed-on-exit). | Medium (closed) | n/a — fix lands in `src/generation/calls.rs`. |
| 204 | Native: tail-expression `return inner_call()` from a struct-returning function emitted `n_inner(cell, args)` as a void STATEMENT and then `return DbRef { store_nr: u16::MAX, rec: 0, pos: 8 };` (null sentinel) — caller's OpCopyRecord then panics on the null sentinel.  **Closed (2026-05-02)** by plan-11: `src/generation/pre_eval.rs::detect_ref_tail_capture` now unspans `Value::Span` wrappers before matching, so the tail-position Call is properly captured via `let __native_tail_ret: DbRef = call(...);` and returned instead of the null sentinel.  Existing tail-capture infrastructure was correct; the walker just didn't see Span-wrapped operators.  Native suite: 91/93 → **93/93** (`85_yield_resume` + `87_store_leaks` closed, `repro_p204.loft` un-marked).  Pinned by `p204_tail_expression_return_passes_under_native` regression test in `tests/codegen_emitter.rs`. | Medium (closed) | n/a — fix lands in `src/generation/pre_eval.rs`. |
| 205 | Native: bounded-generic dispatch `fn f<T: Trait>(x: T) -> text { x.to_label() }` returned a `Str` whose pointer referenced a local `String` that dropped on function return → dangling pointer, comparison failed.  **Closed (2026-05-02)** by plan-09 phase 07: `src/generation/emit.rs` now detects "function returns Type::Text but has no `Type::RefVar(Type::Text(_))` attribute" at both Value::Return and block-tail wrap_result emit sites, and routes the value through `stores.scratch` (the backing String lives as long as `stores` does — same pattern `n_parallel_buf_get_text_native` uses).  Native suite: 89/93 → 90/93 (`86_interfaces` closed).  Pinned by `p205_repro_passes_under_native` + `p205_no_str_new_of_local_in_corpus` regression tests in `tests/codegen_emitter.rs`. | Medium (closed) | n/a — fix lands in `src/generation/emit.rs`. |
| 207 | Native codegen E0308 when comparing a `character` element read from a `Type::Tuple` against a `character` literal.  Reproducer: `t = ('a', 42); if t.0 == 'a' { ... }` — under `--native` rustc rejected the generated `let _v_v1 = (var_t.0); if _v_v1 == char::from(0) { ... } else { i64::from(_v_v1 as u32) }` because `var_t.0` is typed as `i32` (per the tuple element layout) but the null-sentinel comparison uses `char::from(0)` (a `char`).  The non-tuple form (`c = 'a'; if c == 'a'`) compiled fine — the divergence was specific to tuple-element char reads.  **Closed (2026-05-04)** by `src/generation/calls.rs::substitute_template_body` — added a `Value::TupleGet` arm to the existing char-typed-parameter wrap (mirrors the `Value::Var` arm at line 302).  The arg gets `ops::to_char(var_t.0)` wrapping before the template substitutes it for `@v1`, so the `@v1 == char::from(0)` comparison gets a `char`, not the `i32`-typed tuple-element read.  Pinned by `e1_d1_char_int_local` in `tests/tuple_matrix.rs` (un-ignored). | Medium (closed) | n/a — fix lands in `src/generation/calls.rs`. |
| 208 | Native codegen E0282 "type annotations needed" when a bounded-generic method call returns text and the result is used in `+ "!"` text concatenation.  Reproducer: `fn label<T: Printable>(x: T) -> text { x.to_text() + "!" }`.  Surfaced by plan-17 phase 01 (B) follow-up — was masked by the earlier type-inference failure that rejected the snippet at parse time.  Two scratch.push wraps stacked redundantly: the function body block (`Add text_2: text`) had its last operator as a `Value::Return` whose own emit path wraps with `return { stores.scratch.push((expr).to_string()); Str::new(...) }`; the outer block-tail emit then wrapped the same expression in another `{ stores.scratch.push((block).to_string()); ... }`.  The inner `return` made the outer wrap unreachable; rustc rejected the `to_string()` on the never-typed expression with E0282.  **Closed (2026-05-04)** by extending `value_is_return` in `src/generation/emit.rs::output_block` to walk through Block tails — when a Block's last operator is a Return (recursive), the outer `wrap_result` scratch-push wrap is suppressed.  The inner Return handles all return-value semantics on its own.  Pinned by interpreter + native verification of the reproducer. | Medium (closed) | n/a — fix lands in `src/generation/emit.rs`. |
| 209 | Match guard arms with pattern bindings (`x if x < 0 => …`) saw the binding variable as uninitialised because the binding assignment was prepended only to the arm body, not to the guard expression.  Result: `x` read as 0 inside the guard, so `x if x < 0` failed and `x if x == 0` matched everything; interp shifted arms by one, native always returned arm 2.  **Closed (2026-05-04)** by `src/parser/control.rs::parse_scalar_match` — when an arm has both bindings and a guard, the guard is wrapped in a `binding_guard` block whose statements run the bindings before evaluating the guard.  The enum-variant struct-field path at `build_scalar_chain`'s call site already did this correctly; the scalar-match path was the missing case.  Pinned by `tests/issues.rs::p209_scalar_match_guard_sees_pattern_binding`. | High (closed) | n/a — fix lands in `src/parser/control.rs`. |
| 210 | Native coroutine `while … { yield … }` silently returned 0 because `collect_segments` in `src/generation/coroutine.rs` only recognised `Value::Block` containing yields (the for-loop shape) and missed `Value::Loop` (the while-loop shape).  The state machine ended up with no arms, so every `next_i64` call returned `COROUTINE_EXHAUSTED` and the driving for-loop broke immediately.  Interp drives generators via the bytecode VM, not the state-machine lowering, so it was unaffected.  **Closed (2026-05-04)** by extending the matcher to `Value::Block(_) \| Value::Loop(_)`.  Pinned by `tests/issues.rs::p210_native_coroutine_while_yield`. | High (closed) | n/a — fix lands in `src/generation/coroutine.rs`. |
| 211 | Coroutine `yield text` rejected on native: rustc raised E0606 "casting `&'static str` as `i64` is invalid" because the `LoftCoroutine` trait only had `next_i64` and the lowering wrapped every yielded value in `(...) as i64`.  The downstream `OpCoroutineNext` consumer fell through to the `as i32` arm and the assignment coercion appended `.to_string()` to a cast (`as i32.to_string()` → E0425).  Interp drove generators through the bytecode VM, which already supported the size-16 (Str) yield path — the original report's "interp empty stdout" claim was stale by 2026-05-05; interp printed all three names correctly.  **Closed (2026-05-05)** by adding a parallel `next_text` method on `LoftCoroutine` (defaulted to return `STRING_NULL`), routing text-yielding generators to override `next_text` (selected on the function's `Iterator<T>` return type), adding the `coroutine_next_text` runtime helper, and dispatching `OpCoroutineNext` size 16 to it in `src/generation/dispatch.rs`.  Pinned by `tests/issues.rs::p211_coroutine_yield_text` + `p211_coroutine_yield_text_while` and the extended `tests/scripts/51-coroutines.loft` (covers both backends via `tests/native.rs`). | High (closed) | n/a — fix lands in `src/codegen_runtime.rs` + `src/generation/coroutine.rs` + `src/generation/dispatch.rs` + `src/generation/emit.rs` + `src/generation/mod.rs`. |
| 212 | Nested tuple literals (`((1,2), (3,4))`, triply nested, or any tuple containing a tuple) panicked at `src/state/codegen.rs:1527:38` because the inline match in `gen_set_first_at_tos`'s `Type::Tuple` arm had no case for an inner element of `Type::Tuple(_)` — it fell through to the "unsupported elem" panic.  **Closed (2026-05-04)** by extracting per-leaf `OpPut*` emission into a recursive helper `emit_tuple_put_ops` that descends through nested tuples, computing each leaf's absolute slot offset and emitting in reverse to match the depth-first push order used by tuple-literal evaluation.  Pinned by `tests/issues.rs::p212_nested_tuple_literal` + `p212_triply_nested_tuple_literal`. | High (closed) | n/a — fix lands in `src/state/codegen.rs`. |
| 213 | Capturing closures stored in struct fields corrupted host state because `Type::Function` fields allocated only 4 bytes (just the function id) but capturing-closure values are 16 bytes (4B d_nr + 12B closure DbRef pointing at a closure record holding captured vars).  Initially closed via parse-time diagnostic; **fully closed (2026-05-05)** by the layout-widening fix (design v4): each fn-ref struct field that ever receives a capturing-lambda assignment is split into TWO database fields — `<attr>` (4B int holding the lambda's d_nr) and `<attr>__closure_rec` (`Parts::ChildRec(closure_kt)` — 4B u32 rec-id of a closure record co-located in host's Store).  Two new ops: `OpClaimChildRec` claims a fresh record in host's Store and deep-copies the closure record's payload + nested heap fields into it; `OpRefFromChildRec` reads the rec-id and constructs the appropriate DbRef.  Free / cross-store-move is automatic via `Parts::ChildRec(content)` cascade arms in `copy_claims` / `remove_claims`.  Non-capturing fn-ref fields and tuple-of-fn-ref elements stay at the original 4B layout (no `__closure_rec`), keeping container layouts unchanged.  Pinned by `tests/issues.rs::p213_struct_field_basic_int` + `p213_struct_field_multi_int_capture` + `p213_struct_field_default_init`.  Text-returning capturing closures via struct field is a separate native-codegen issue tracked as P227 (out-of-scope of P213's layout fix). | High (closed) | n/a — fix lands in `src/database/mod.rs` (Parts::ChildRec), `src/database/allocation.rs` (cascade), `src/database/structures.rs` (claim_child_rec / ref_from_child_rec), `src/typedef.rs` (Type::Function arm), `src/parser/mod.rs` (set_field_check + get_val), `src/fill.rs` + `default/01_code.loft` (new ops). |
| 215 | **Closed alongside P213.** Captured fn-typed local var inside another closure body shared P213's layout limitation; both fully closed by the v4 layout fix. | (closed alongside P213) | n/a |
| 214 | **Non-capturing** vector-of-closures (`vector<fn(integer) -> integer> = [|x| {x+1}, |x| {x*2}]`) panicked under interp (`fn_call_ref: d_nr=12884901896 out of range` — d_nr read from offset 0 of every slot because vector stride was 0) and rejected on native with E0605 (`DbRef as (u32, DbRef)`).  **Closed (2026-05-05)** with three coordinated changes: (1) `parser/fields.rs::parse_index_apply` falls back to `narrow_vector_content` for `Type::Function` element types so `elm_size` is 4 (the d_nr stride) instead of 0; (2) the same path adds a `Type::Function` branch that reads the d_nr via `OpGetInt4` and pairs it with `OpNullRefSentinel` (closure DbRef half), assembling the (u32, DbRef) tuple via the existing `fn_ref_field_read` block-name shortcut in native codegen; (3) `generation/mod.rs::emit_field` emits `db.vector(narrow_int)` for fn-ref vector content instead of `db.vector(u16::MAX)` so runtime parent-tracking finds the int content type.  Pinned by `tests/issues.rs::p214_vector_of_noncapturing_closures`.  Capturing closures in vectors remain deferred (would need 16B-per-element layout — same general direction as P213's `Parts::ChildRec` for struct fields). | High (closed) | n/a — fix lands in `src/parser/fields.rs` + `src/generation/mod.rs`. |
| 215 | Closure-typed local var unreachable from inside another closure body.  Calling `inner(y)` where `inner = fn(x:integer)->integer{…}` from inside `outer = fn(y:integer)->integer{ inner(y) + 1 }` rejected with "Unknown function inner".  **Closed (2026-05-05)** with three coordinated changes: (1) `parser/control.rs::try_fn_ref_call` extended to scan `capture_context` for `Type::Function` names, mirroring the bare-name capture path in `parser/objects.rs:162-200` — pushes to `captured_names`, creates a placeholder local var, and at emit-time wraps the CallRef in `Set(v_nr, get_field(closure_param, ...))` to load the captured fn-ref from the closure record at the call site.  (2) `parser/mod.rs::emit_fn_ref_field_write` lifts the P213-deferred "only inline lambda literals" diagnostic when both target and source are non-capturing (target field has 4B int layout AND source var is not in `closure_vars`); emits `OpSetInt4(target, pos, Value::FnRefDnr(src))` to project the d_nr.  (3) `parser/mod.rs::get_field` handles 4B-int-layout fn-ref reads by synthesising a null DbRef for the closure half via `OpNullRefSentinel` instead of reading at `pos+4` (which would corrupt the next attribute — the legacy 4B layout has no `__closure_rec` half).  New IR variant `Value::FnRefDnr(u16)` projects the d_nr from a fn-ref Var: interp via `OpVarInt(slot_pos)` (`fill.rs::var_int` reads 8B regardless of declared type), native via `(var_<name>.0 as i64)`.  Pinned by `tests/issues.rs::p215_nested_closure_call` + `p215_multiple_captures_in_one_closure`.  Capturing source lambdas (e.g. `inner` itself captures) remain deferred — would need `synthesize_closure_record` to register the 8B split layout when the captured lambda itself captures (recursive closure-record allocation chain). | High (closed) | n/a — fix lands in `src/parser/control.rs`, `src/parser/mod.rs`, `src/data.rs`, `src/state/codegen.rs`, `src/generation/emit.rs`. |
| 216 | Tuple-capture in closure body crashed under interp ("Incomplete record" at `src/store.rs:227`) and produced wrong results under native (read `t.1` for `t.0` reads — all writes went to the same `pos + u16::MAX` offset, so the last write won and both reads returned the same garbage).  Root cause: `synthesize_closure_record` (`src/parser/vectors.rs:762`) added a `Type::Tuple` attribute to the closure record but never registered the synthetic `__tuple<…>` struct via `data.tuple_def(...)`.  `fill_database` then saw `type_elm(&Type::Tuple(_))` return `u32::MAX` and silently skipped the attribute (line 381 gate), leaving the closure record with size 0 → `OpDatabase` panicked claiming an empty record.  **Closed (2026-05-05)** in `src/parser/vectors.rs::synthesize_closure_record` by walking each capture's `Type` and calling `tuple_def` for every `Type::Tuple` (recursively, so nested tuples register inside-out) before adding the attribute.  The new helper `ensure_tuple_defs_for_capture` also recurses through `Type::Vector` / `Type::RefVar` wrappers so a `vector<(int,int)>` capture works.  Pinned by `tests/issues.rs::p216_tuple_capture_int_first` + `p216_tuple_capture_int_second` + `p216_tuple_capture_three_elements`. | High (closed) | n/a — fix lands in `src/parser/vectors.rs::synthesize_closure_record`. |
| 229 | Two `tests/multiplayer_v2.rs` scenarios failed on specific OS CI runners.  **(a) `v2_two_clients_with_spectator_routing` (macOS)**: scheduler ran both clients so fast that they completed their 3 X moves with no observable overlap.  **Closed (2026-05-06)** by adding `n_sleep_ms` to `lib/web/native` (exposed as `web::sleep_ms`); the v2 client now honours `LOFT_TICTACTOE_CLIENT_DELAY_MS` for a one-time post-handshake pause; the multi-client test sets it to 200 ms so both clients are registered before either makes a move.  Linux still passes; macOS now deterministic (un-ignored).  **(b) Windows CI** still gates the suite (`wait_listening` timed out at exactly 60 s on both retries, suggesting the v2 server child died at startup).  No Windows reproduction available locally; **diagnostics improved (2026-05-06)**: `ServerGuard::diagnose_listen_failure` now drains the child's stdout/stderr and reports its `try_wait` status when the listen times out, replacing the bare "60s exceeded" panic with actionable signal.  Three Windows-ignored scenarios remain marked `#[cfg_attr(target_os = "windows", ignore = "P229b: …")]` so the next Windows CI run produces a real error message that can be diagnosed remotely. | (a) Closed; (b) Open (Windows) | Linux passes 3/3 with `--test-threads=1`; Windows: the next CI run will surface the actual server-side failure via `diagnose_listen_failure` in the panic message. |
| 230 | `yield` inside a conditional block (`if`, `else`) within a generator emitted a raw Rust `yield` keyword on native (E0627 "yield expression outside of coroutine literal"), instead of translating into a state-machine return.  Interp worked because the bytecode VM doesn't use Rust's unstable `gen` syntax.  Root cause: `src/generation/coroutine.rs::collect_segments` only matched yields at the TOP LEVEL of a generator body's operator list (Simple / YieldFrom / ForLoopBody segments) — `Value::Block` and `Value::Loop` containing yields were already routed to ForLoopBody, but `Value::If` was missed.  An `if`-with-yield fell through to the `pre` accumulator, then `output_code_inner` hit `Value::Yield` and emitted literal `yield ...` Rust syntax.  **Closed (2026-05-06)** by extending the matcher to `Value::Block(_) \| Value::Loop(_) \| Value::If(_, _, _)`.  The eager-collect factory's `yield_collect = true` mode then emits `__values.push(...)` for the conditional yield (mirroring how Block-with-yield was already handled).  `contains_yield` already walked through `Value::If` so detection works without further changes.  Pinned by `tests/issues.rs::p230_yield_in_if_block` + `p230_yield_in_else_branch` + `p230_yield_in_both_branches_with_trailing_simple` and the reproducers in `/tmp/p_followups/p230_yield_in_if.loft` + `_in_else.loft`.  Note: `match` arms with yields remain a separate parse-time rejection, NOT covered by this fix. | Medium (closed) | n/a — fix lands in `src/generation/coroutine.rs::collect_segments`. |
| 231 | `cargo test --release --test multiplayer_v2` failed 0/3 (or 1/3, depending on host CPU count) on default cargo parallelism.  All three tests' `ServerGuard::spawn` invocations tried to bind port 7878, which the v2 server hardcoded at `lib/game_protocol/examples/tictactoe_server_v2.loft:313`.  The second and third `loft_tcp_listen` calls failed to bind (address in use), but `wait_listening` still succeeded because it connected to the FIRST test's already-running server — which then reported "server-full" to the colliding test's clients.  **Closed (2026-05-06)** with three coordinated changes: (1) `tictactoe_server_v2.loft` reads `LOFT_TICTACTOE_PORT` (default 7878) via `env_variable`; (2) `tictactoe_client_v2.loft` reads the same env var to target the per-test server URL; (3) `tests/multiplayer_v2.rs` adds `pick_free_port()` (binds `127.0.0.1:0`, drops, returns the kernel-chosen port) and threads a unique port through every `ServerGuard::spawn` + `spawn_client*` site, forwarding via `LOFT_TICTACTOE_PORT`.  Surfaced and required fixing P232 (`env_variable` dangling-Str) first — without it the env var read returned garbage bytes.  After all edits the default `cargo test` parallelism passes 3/3 reliably.  Pinned by the existing 3 `multiplayer_v2` tests now running in parallel without collision. | Low (closed) | n/a — fix lands in `lib/game_protocol/examples/tictactoe_server_v2.loft`, `lib/game_protocol/examples/tictactoe_client_v2.loft`, and `tests/multiplayer_v2.rs`. |
| 232 | `env_variable(name)` returned a `Str` whose pointer dangled into a dropped local `OsString` (native build) or `String` (WASM build).  `Stores::os_variable(name: &str) -> Str` was a static-style function with no `&mut self`: it constructed `Str::new(v.to_str().unwrap())` over a local `v: OsString`, returned, and the caller saw garbage bytes — `env_variable("MYVAR")` reading `"hello"` came back as random non-UTF8 sequences, `as integer` then produced `null`.  Latent because the in-tree test (`tests/scripts/19-files.loft:99`) only exercised the unset case (where the empty-string path bypassed the dangling branch).  Surfaced 2026-05-06 while wiring `LOFT_TICTACTOE_PORT` for P231.  **Closed (2026-05-06)**: `Stores::os_variable` now takes `&mut self` and pushes the resolved value into `self.scratch`, returning a `Str` that borrows from the persistent buffer (mirrors the P205 pattern).  `n_env_variable` (interp bridge) and the `#rust"stores.os_variable(@name)"` template (native codegen) both updated to the new instance-method form.  Pinned by `tests/issues.rs::p232_env_variable_round_trips_set_value` — sets a process-unique env var, reads it back through `env_variable` in loft, asserts byte-exact equality. | Medium (closed) | n/a — fix lands in `src/database/format.rs::Stores::os_variable` + `src/native.rs::n_env_variable` + `default/02_images.loft` (#rust template). |
| 233 | `tests/testing.rs::code!()` test harness hung the loft parser/lexer when the expected `Value::Text` value contained JSON-escape sequences (`\"`, `\\`).  Root cause: `replace_tokens` (the function that prepares `Value::Text` for embedding into a generated loft `assert(...)` script) escaped `{`, `}`, real newlines, and `"` — but NOT `\`.  A lone `\` in the expected text survived the encoding intact; once the text was wrapped in `"..."` and parsed by loft's lexer, the lexer interpreted that backslash as an escape introducer.  For shapes like `\\"` (3 chars after step 4 of the original `replace_tokens`), the lexer read `\\` → `\` then encountered the bare `"` and closed the string literal mid-scan, leaving the lexer in a recovery state that looped indefinitely on subsequent characters of the assertion's format-string body.  Surface symptoms (PR #210 CI, 2026-05-07): macOS `q3b_struct_to_json_string_escapes_control_chars` FAILED with a round-trip mismatch; Ubuntu `q3b_struct_to_json_string_escapes_quote_and_backslash` then HANGED indefinitely (2h+ on the runner).  Same loft source run STANDALONE via `--interpret` did NOT hang; the harness's `Drop for Test` assembly was the trigger.  **Closed (2026-05-07)**: `replace_tokens` now escapes `\` first (`\` → `\\`), before the existing `{` / `}` / real-newline / `"` steps.  Order matters — the later steps add fresh `\` characters as part of canonical escape sequences (`\n`, `\"`) that should NOT be re-escaped.  Pinned by the new Rust-level `tests/testing.rs::replace_tokens_tests` module (8 round-trip property tests covering ASCII passthrough, quoted strings, the previously-hanging double-backslash and `\"` shapes, JSON-escaped text from the q3b expected values, real newlines, real tabs, curly braces).  The two q3b loft-surface tests dropped on PR #210 (`q3b_struct_to_json_string_escapes_quote_and_backslash` + `_control_chars`) are re-enabled — both now pass via the `code!()` harness without hangs.  Per-byte JSON escape dispatch stays locked at the Rust unit level by `src/database/format.rs::json_escape_tests` (13 tests).  Lexer-side hard-cap on string scanning (fix shape b in the original report) was NOT done — the harness fix is sufficient for the surface-level symptom; a true lexer-loop reproducer outside the harness's specific assembly would need to surface separately. | Medium (closed) | n/a — fix lands in `tests/testing.rs::replace_tokens` + new `tests/testing.rs::replace_tokens_tests` module + re-enabled `tests/issues.rs::q3b_struct_to_json_string_escapes_*` tests. |
| 228 | `t = ("hello", 42); label = t.0;` — assigning a text-typed tuple element to a `text` local rejected on native with E0308.  The emitted Rust was `let mut var_label: String = &var_t.0.to_string();` — `&var_t.0.to_string()` parses as `&(var_t.0.to_string())` per Rust method-call precedence, producing `&String` against a declared `String`.  Surfaces with or without a closure capture.  Interp was unaffected.  **Closed (2026-05-06)**: `src/generation/dispatch.rs::output_set` had a `tuple_text_elem_clone` detection (added by T1.8a) that recognised `Value::TupleGet(v, idx)` on the RHS and emitted `var_t.0.clone()` to bypass the precedence trap — but the `matches!(to, Value::TupleGet(...))` predicate didn't unwrap the `Value::Span` wrapper that the parser puts around every assignment RHS, so the fast-path never fired in practice and codegen fell through to the buggy `&...to_string()` form.  Fix unspans `to` before pattern-matching at both the detection site and the matched-clause read; also extends the symmetric `text_local_clone` `Value::Var` detection to unspan for the same reason.  Pinned by `tests/issues.rs::p228_text_tuple_element_assignment` + `p228_text_tuple_element_at_higher_index`. | Medium (closed) | n/a — fix lands in `src/generation/dispatch.rs::output_set`. |
| 217 | Text accumulator pattern broken on both backends: `out = "x"; out = out + "y";` produced `"xxy"` on interp and rejected on native with E0502.  Native codegen emitted `*var_out += &*(&*var_out); *var_out += &*("y");` — the LHS was appended to itself before the literal was concatenated.  The `assign_text` self-append detector in `src/parser/operators.rs` *did* exist for this case but compared `args[1] == Value::Var(var_nr)` with literal equality, which missed the `Value::Span(Var(var_nr))` operand the parser wraps for source-position tracking; the RefVar(Text) path through `append_to_text` in `src/parser/expressions.rs` had no detection at all.  **Closed (2026-05-05)** by extending the detection to unspan both `args[0]` and `args[1]` before the structural compare in `assign_text`, and copying the same detection into `append_to_text` so the RefVar(Text) parameter path (text-returning functions) gets the same fix.  Pinned by `tests/issues.rs::p217_text_self_accumulator` + `p217_text_accumulator_chain` and the existing `tests/scripts/29-strings.loft::test_text_self_concat` / `test_text_self_concat_loop` tests (now actually pass; were silently failing on `--native` and giving the duplicated-content garbage on interp).  Two narrow shapes still open as separate P-issues: P222 (`s = s + s` native E0502) and P223 (`s = "literal" + s` self-prepend — both backends). | High (closed) | n/a — fix lands in `src/parser/operators.rs` + `src/parser/expressions.rs`. |
| 222 | `s = s + s` rejected on native with E0502 "cannot borrow `*var_s` as mutable because it is also borrowed as immutable".  Interp already gave the correct answer (`s = "ab"; s = s + s` → `"abab"`) after the P217 fix.  Same root-cause family as P217 — after the P217 self-append-strip the IR becomes `OpAppendText(s, Var(s))`, and the native emitter lowered it to `var_s += &*(&var_s);` (self-borrow).  P217's fix didn't reach this case because `s + s` has TWO references to `s`: the first is detected and stripped (good), but the second `Var(s)` survives.  **Closed (2026-05-06)** in `src/generation/text.rs::append_text` — when the RHS expression references the destination variable (detected via `code_references_var`), the value is hoisted through a fresh `String` (`let __p222_tmp: String = ...; var_s += &__p222_tmp;`) so the self-borrow never overlaps the `+=` target.  Pinned by `tests/issues.rs::p222_text_self_double` + `p222_text_triple_self_reference` and the new `tests/scripts/29-strings.loft::test_text_self_double` (covers both backends via `tests/native.rs`). | Medium (closed) | n/a — fix lands in `src/generation/text.rs::append_text`. |
| 223 | Self-prepend `s = "literal" + s` (and any literal-first concat where the destination appears on the RHS) was broken on both backends.  Two compounding bugs: (a) `parse_operators` captured `orig_var = s` BEFORE the recursive parse filled `code` — for a literal-first concat, `code` ended up as `Text("lit")` but `orig_var` still pointed at `s`, so `parse_append_text` used `s` as the accumulator and emitted `OpAppendText(s, "lit")` as the first op; combined with `assign_text`'s pre-clear, this destroyed `s`'s original content.  (b) `code_references_var` (used by `assign_text` to decide when to wrap the RHS in a protective work-text) didn't walk `Value::Block` — the work-text Block produced by `parse_append_text` for `"lit" + var` carries `Var(var)` deep inside, so the wrap was skipped and the interpreter's clear-before-evaluate text-Set semantics destroyed the content before reading.  **Closed (2026-05-05)** with four coordinated changes: (1) `parse_operators` only passes `orig_var` to `parse_append_text` when `code.unspan()` still equals `Var(orig_var)` after recursion, otherwise falls back to `u16::MAX`.  (2) `code_references_var` walks `Value::Block`.  (3) `Parser::append_to_text` (RefVar(Text) parameter path) gained the same self-reference wrap as `assign_text`.  (4) Native codegen's `Set(RefVar(Text), …)` emission wraps the RHS in parens before appending `.to_string()` to fix Rust method-call precedence.  Pinned by `tests/issues.rs::p223_self_prepend_local_text` + `p223_self_prepend_in_text_returning_fn` and `tests/scripts/29-strings.loft::test_text_self_prepend`. | High (closed) | n/a — fix lands in `src/parser/operators.rs` + `src/parser/expressions.rs` + `src/generation/dispatch.rs`. |
| 224 | Coroutine yields a value that depends on a function-local variable (declared INSIDE the generator body, not a parameter) — native rejected with E0425.  Both the integer and text shapes failed because IR `Set(local, …)` lowered to `let mut var_X = …` inside one match arm, scoping the binding to that arm only and losing the value across `next_*` calls.  **Closed (2026-05-05)** by promoting non-argument coroutine-body locals (primitive + text types) to fields on the generator struct so writes from one state arm survive into the next.  `coroutine_persistent_locals` (`src/generation/coroutine.rs`) collects qualifying vars; `emit_struct_def` adds a field per local; `emit_factory_fn` initialises each to its default value; `output_set` and the `Value::Var` arm in `output_code_inner` route reads/writes through `self.var_X` for vars in the new `Output.coroutine_persistent_vars` set.  Pinned by `tests/issues.rs::p224_coroutine_local_int_capture` + `p224_coroutine_local_text_capture`.  References intentionally out of scope (would need Store-allocation cascade in the factory). | High (closed) | n/a — fix lands in `src/generation/coroutine.rs` (persistent-locals collection + struct/factory wiring), `src/generation/dispatch.rs` (Set interception), `src/generation/emit.rs` (Var interception), `src/generation/mod.rs` (`Output.coroutine_persistent_vars`). |
| 225 | `yield from` mixed with `Simple` yields in the same generator produced duplicate output of the first yield on native.  Root cause: the `Value::Block`-with-yield-from segment fell through to the `ForLoopBody` classifier (the strict `detect_yield_from` shape didn't match), which made `has_for_body` true.  The factory's `emit_for_body_factory` then eager-collected EVERY segment (Simple, YieldFrom, ForLoopBody) into `__values`, but `emit_next_i64` ALSO emitted per-segment match arms that re-yielded Simple values via `return val`.  Result: state 0 returned `"start: hi"` directly, state 1 popped `__values[0]` which was also `"start: hi"`, etc.  **Closed (2026-05-05)** by collapsing the impl to a single pop-from-buffer arm whenever any segment is `ForLoopBody` — the factory owns all the work in that case, the impl just drains the buffer.  Pinned by `tests/issues.rs::p225_yield_from_mixed_with_simple_yields`. | High (closed) | n/a — fix lands in `src/generation/coroutine.rs::emit_next_i64`. |
| 218 | Format string interpolating a parameter inside a coroutine generator body declared the `__work` text-format buffer inside state 0's match arm via `let mut var___work_N: String = …`, scoping the binding to that arm only.  Every later state arm referencing the same buffer (a sibling yield with another format string) failed to compile with E0425 ("cannot find value `var___work_2` in this scope").  The same shape applied to `while` bodies that yield format strings — the eager-collect factory `emit_for_body_factory` has its own emission path with the same first-use-declares-locally pattern.  **Closed (2026-05-05)** in `src/generation/coroutine.rs` by pre-declaring `__work_*` text locals at function scope before the per-state code in BOTH `emit_next_i64` (state-machine method body) and `emit_for_body_factory` (eager-collect factory used for for/while bodies with yields), then marking them in `self.declared` so the IR's per-state `Set(__work_N, "")` ops emit as plain assignments.  Pinned by `tests/issues.rs::p218_coroutine_yield_format_with_param` + `p218_coroutine_while_yield_format` and the extended `tests/scripts/51-coroutines.loft` (covers both backends via `tests/native.rs`). | High (closed) | n/a — fix lands in `src/generation/coroutine.rs::emit_next_i64` + `emit_for_body_factory`. |
| 219 | Vector-element ForLoopBody in a generator body emitted invalid Rust for the eager-collect factory: `return 'l4: loop {…}` (E0308 — unit vs `Box<dyn LoftCoroutine>`).  Root cause: scopes' `insert_free` adds `Return(Null)` at the end of the function body block; `patch_hoisted_returns` Pass 2 in `src/generation/pre_eval.rs` coalesces `[…, Loop(…), Return(Null)]` into `[…, Return(Loop(…))]`; `Value::Return(Loop)` then emits as `return 'l4: loop {…}`.  Range-for didn't trip this because its IR shape doesn't include a top-level `Loop` operator that the patch can pair.  **Closed (2026-05-05)** in `src/generation/coroutine.rs::emit_for_body_factory` by stripping trailing `Return` ops from the body's operator list before `generate_expr_buf` runs — the factory drives the body purely for its yield side effects (populates `__values`); the actual factory return is `Box::new(struct_name { … })` emitted separately.  Pinned by `tests/issues.rs::p219_vector_for_yield_in_generator` + `p219_vector_for_yield_text`. | Medium (closed) | n/a — fix lands in `src/generation/coroutine.rs::emit_for_body_factory`. |
| 226 | Vector literals (`[1,2,3]`) inside a state-machine generator (Simple-yield-only body — no for-loop, so the eager-collect path doesn't engage) allocated a `__vdb_*` `DbRef` slot that the per-state codegen declared inside one match arm via `let mut var___vdb_N: DbRef = stores.null_named(...)`, scoping the binding to that arm only.  A subsequent state arm referencing the same `__vdb_N` (e.g. two `Simple` yields each containing a vector literal) failed with E0425 "cannot find value `var___vdb_N` in this scope".  Same scoping family as P218 (`__work_*` text buffers) and P224 (general user locals).  The originally documented mixed-Simple+ForLoopBody shape was no longer affected by 2026-05-06 — the `[10,20]` literal in a for-loop body is owned by the eager-collect factory, not by a state arm.  The real surfaceable shape is `yield [1,2,3].len(); yield [10,20,30].len();` (or via locals).  Interp was unaffected (bytecode VM scope is per-function, not per-arm).  **Closed (2026-05-06)** in `src/generation/coroutine.rs::emit_next_i64` by extending P218's pre-declaration list to include `__vdb_*` locals — pre-declared at function scope as `let mut var___vdb_N: DbRef = stores.null_named(...)` and added to `self.declared` so the IR's per-state Set ops emit as plain assignments.  Pinned by `tests/issues.rs::p226_vector_literal_in_yield_across_simple_arms` + `p226_vector_literal_via_local_across_simple_arms`. | Medium (closed) | n/a — fix lands in `src/generation/coroutine.rs::emit_next_i64`. |
| 227 | Text-returning fn-ref calls crashed both backends regardless of where the fn-ref lived (local, struct field, parameter) or whether the lambda captured.  Two independent root causes: (a) **Interp**: `parser/control.rs:3052` (and sibling sites) allocated work-buffers as `(0..deps.len())` where `deps = []` for fn-ref types ⇒ zero buffers pushed, lambda's stack slot for `__work_1` read garbage ⇒ SIGSEGV.  (b) **Native**: dispatch wrapper allocated `_fnref_work` block-scoped, so the lambda's returned `Str` borrowed a buffer that dropped before the outer `.to_string()` read its bytes ⇒ `ptr::copy_nonoverlapping` UB.  **Closed (2026-05-05)** with three coordinated changes: (1) parser allocates exactly ONE `work_text` var per text-returning fn-ref call site (3 sites in `parser/control.rs` + `parser/operators.rs`) — replaces the deps-derived count with a fixed 1 when ret is `Type::Text`; (2) `text_return` (`parser/control.rs:2405`) ensures every text-returning lambda has at least one `RefVar(Text)` hidden attribute so the fn-ref ABI is uniform — even constant-return lambdas (`fn() -> text { "hello" }`) get a canonical `__work_ret` slot via `add_attribute` + `create_var` + `become_argument`; (3) native `output_call_ref` (`src/generation/emit.rs`) detects hidden attrs by TYPE (`Type::RefVar(Type::Text)`) instead of name (work-buffer attrs are named after user-shadowed text vars like `a`/`b`, not `__work_*`), strips the trailing work-buffer arg from candidate matching, threads the parser-injected `&mut String` into the dispatch arm via `_farg_<n>`, and wraps each arm's result with `.to_string()` to unify heterogeneous candidate Rust signatures (some return `Str`, some return `String`).  Pinned by `tests/issues.rs::p227_text_fn_ref_local_call` + `p227_text_fn_ref_local_with_capture` + `p227_text_fn_ref_struct_field` + `p227_text_fn_ref_struct_field_capture`.  Side-effect of widening the dispatch: `Stores::source_dir_native()` (declared in `default/03_text.loft`) had a missing Rust impl that was never compiled before — added a stub returning `String::new()` in `src/database/format.rs`; full implementation is tracked separately. | High (closed) | n/a — fix lands in `src/parser/control.rs`, `src/parser/operators.rs`, `src/generation/emit.rs`, `src/database/format.rs`. |
| 220 | `""` literals stored into a `vector<text>` then deep-copied (struct field assignment, `vector_add`, `OpCopyRecord`, parallel-worker boundary, etc.) silently re-classified as `null` on the destination.  Both backends agreed on the wrong output, so the bug was in shared code: `Stores::copy_claims` in `src/database/allocation.rs` checked the *string content* via `s.is_empty()` to decide whether to allocate or write the null sentinel.  `get_str(0)` returns `STRING_NULL` (`"\0"`, len 1) for a null source, so the empty-content check never fired for genuine nulls — but it DID fire for genuinely empty `""` sources, writing 0 (the null sentinel) and re-classifying the value as null.  **Closed (2026-05-05)** by discriminating on the source `cur` field (non-zero = allocated, regardless of string length): nulls stay null, `""` round-trips as `""`.  Pinned by `tests/issues.rs::p220_empty_string_in_vector_text_round_trips_through_struct_field` + `p220_null_text_preserved_through_struct_field`.  Surfaced during TIC_TAC_TOE v2 development (commit `113dc8a`); the comment in `lib/game_protocol/examples/tictactoe_server_v2.loft:53-56` recorded the workaround discovery. | High (closed) | n/a — fix lands in `src/database/allocation.rs::copy_claims`. |
| 221 | Server-side HTTP `parse_request` in `lib/server/native/src/lib.rs:27` used `BufReader::new(stream)` and let the BufReader drop at end of function.  Same bug class as the client-side handshake bug fixed in `lib/web/native/src/ws_client.rs` in commit `113dc8a`: if the client sent WS frames immediately after the upgrade request without waiting for the server's `101` response, the BufReader's internal buffer absorbed the body bytes AND the leading WS frame bytes; on drop, the WS bytes were lost.  Latent with stock browser traffic (browsers wait for `101`), but a custom client trips it.  **Closed (2026-05-06)**: `parse_request` now reads the header block byte-by-byte until `\r\n\r\n` (mirroring `ws_client.rs:180-220`); the body is read directly from the `&mut TcpStream`.  Post-header bytes stay in the kernel buffer for the next reader.  Pinned by `lib/server/native/src/lib.rs::tests::p221_parse_request_leaves_post_header_bytes_in_kernel_buffer` — runs an aggressive client that writes the upgrade request and a trailing payload back-to-back, then asserts the trailing bytes are still readable from the server's stream after `parse_request` returns. | Medium (closed) | n/a — fix lands in `lib/server/native/src/lib.rs::parse_request`. |
| 234 | Tuple-of-struct member access — `tuple.0.field` syntax — failed the lexer with "Problem parsing float" on the `0.field` substring (the lexer treated `0.f...` as the start of a float literal); separately, function-return tuples containing a Reference / Vector / Enum-struct element (e.g. `fn make() -> (Point, integer)`) corrupted at the call boundary on `--native` (`r.1=0`, `r.0.x=null`).  Surfaced 2026-05-07 during the ARC.md A7 actual-error survey on `par_tuple_return_struct_text`.  **Lexer half closed (2026-05-07)** in `src/lexer.rs::number` — extended P195's `prev_was_field_dot` branch to fire regardless of what follows the second `.` (digit, identifier, anything).  Pinned by `tests/lexer::test::p234_tuple_index_then_field_does_not_glue_into_float`.  **Runtime half closed (2026-05-08)** by routing tuple-with-lifetime-concern returns through the existing synthetic `__tuple<…>` struct (Plan-14 phase 07).  `src/parser/definitions.rs::parse_function` rewrites the function's `returned` from `Type::Tuple(elems)` to `Type::Reference(tuple_def(elems))` whenever any element has a lifetime concern (Text, Reference, Vector, Enum-struct, keyed collection, RefVar, or a nested tuple containing one — predicate `data::has_lifetime_concern`).  `src/parser/control.rs::block_result` then detects body-tail `Value::Tuple(...)` literals and rewrites them via `rewrite_tail_tuple_to_synthetic_struct` into the same allocation + per-field-init sequence inline struct literals produce.  All existing struct-return ownership-transfer machinery (`ref_return`, `text_return`, `OpDatabase`, `OpGetField`) applies unchanged.  Pure-value tuples (`(integer, integer)` etc.) keep the Rust tuple ABI; T1.8a's `(text, text)` text-tuple machinery in `src/generation/{mod.rs, emit.rs, dispatch.rs}` becomes superseded but kept as defensive fallback.  Pinned by `tests/issues.rs::p234_runtime_*` + un-ignored `tests/threading_chars::par_tuple_return_struct_text` canary.  Side benefit: closes ARC.md A7.3 (`par_tuple_return_struct_text` was the canary blocked on this exact bug). | High (closed) | n/a — fix lands in `src/data.rs` (`has_lifetime_concern` predicate), `src/parser/definitions.rs` (return-type rewrite), `src/parser/control.rs` (body-tail rewrite + helper). |
| 235 | For-loop tuple destructure `for (a, b) in pairs { ... }` rejected with "Expect variable after for" — even WITHOUT par.  Surfaced 2026-05-07 during the ARC.md A7 actual-error survey on `par_tuple_destructure_in_for`.  **Non-par half closed (2026-05-07)** in `src/parser/collections.rs::parse_for` — parser now accepts `(name1, name2, …)` after `for`, synthesizes a temp loop var named `__destructure_t_<line>_<pos>`, defines each user-named binder as a proper variable typed as the matching tuple element, and prepends `Set(name_i, get_val(loop_var, offset_i))` ops to the body so each iteration unpacks the tuple before the user code runs.  Handles both direct `Type::Tuple([…])` (rare) and the common `Type::Reference(__tuple<…>)` shape (vector<(T1,T2)> iteration via P189b's element-access path).  **Par half STILL OPEN**: `for (a, b) in pairs par(r = work(a, b), 4) { … }` requires a synthesized wrapper worker — the existing par dispatch passes ONE per-iteration arg (the loop element) and N context args (same every iteration), but destructure wants TWO per-iteration args derived from the tuple.  Cleanest fix for the par half: at parse time, when destructure is paired with par, synthesize a wrapper `__par_destructure_w<N>(t: tuple_type) -> ret { worker(t.0, t.1) }` and rewrite the par expression to call the wrapper with the tuple loop element.  Blocks `par_tuple_destructure_in_for` (still ignored with this P-id reference).  Estimated effort for the par half: M (~1 session). | Low-Medium (no functional gap — workaround exists for both halves) | (a) Non-par: now works directly.  (b) Par: write a wrapper fn `fn pair_add(p: (T1,T2)) -> R { work(p.0, p.1) }` and use `for p in pairs par(r = pair_add(p), N)`. |
| 236 | A function whose return type is a heap-owned reference (`Reference`, `Vector`, struct-enum) and whose body's tail expression is an `if/else` (or `match`) returning newly-constructed records corrupted the return value on `--native` (returned the typed null sentinel `DbRef { store_nr: u16::MAX, rec: 0, pos: 8 }` instead of the if/else's value).  Interpreter worked because `OpReturn(value=12)` reads from the eval-stack top (which the if/else branches populate); native generated `if cond { …; w } else { …; w }; OpFreeRef(w); return DbRef::null();` and the if/else's value was dropped on the floor.  Reproducer: `fn make_p(c: bool) -> P { if c { P{x:1, y:2} } else { P{x:3, y:4} } }` — interp printed `(1, 2)`, native printed `(null, null)`.  Verified pre-existing on `eebaaa4`.  Surfaced 2026-05-08 during ARC.md A7.1 (par tuple wide-return) when widening the parse_function gate to route pure-value tuple returns through `Reference(__tuple<…>)` exposed the same shape — `min_max(...) -> (integer, integer) { if cond { (a, b) } else { (b, a) } }` regressed `tests/docs/28-tuples.loft`.  **Closed (2026-05-08)** by three coordinated changes: (a) `parser/control.rs::unify_if_branches_work_refs` walks the body's tail If/Match (else-if chains lower to nested `Value::If`) and rewrites the false branch's terminal work-ref `Var` references via the new `substitute_work_ref` helper so both branches end with the SAME `Var(shared_w)`; gated to fire only when both terminal vars match `__ref_*` / `__rref_*` naming so user-named parameters are never renamed (regression caught when `gen_max<T: Ordered>(gen_x: T, gen_y: T)` accidentally returned `gen_x` for both branches).  (b) `scopes.rs::returned_var` recurses through `Value::If` and reports the shared var when both branches match, so `get_free_vars` skips OpFreeRef on it; `ref_return` then promotes it to a hidden caller arg as for single-branch reference returns.  (c) `scopes.rs::free_vars` catchall emits `Return(Var(ret_var))` instead of the legacy `Return(Null)` (which depended on the bytecode VM's eval-stack-top semantics — broken under native).  Same path closes the tuple variant: `parser/control.rs::rewrite_tail_tuple_to_synthetic_struct` recurses through If/Block/Insert with a SHARED work-ref via `rewrite_tail_tuple_with_work_ref`, so the synthetic-struct construction sequence reuses one DbRef across all branches.  Pinned by `tests/issues.rs::p236_struct_return_from_if_else_{true,false}` + `_else_if_chain` + `_param_returning_if_unaffected` (regression guard for the parameter-substitution gate) + `_tuple_return_from_if_else` + the previously-blocked `tests/threading_chars::par_tuple_return_{int_int,three_arity,nested}` canaries (un-ignored).  Side benefit: closes ARC.md A7.1 entirely. | High (closed) | n/a — fix lands in `src/parser/control.rs` (unify helper + tuple recursive rewrite + tail_has_tuple_leaf gate), `src/scopes.rs` (returned_var recursion + Return-with-ret-var emission), `src/parser/definitions.rs` (size-based tuple_rewrite gate widen), `src/parser/expressions.rs` (Reference(__tuple<...>) destructure arm). |

## Interpreter Robustness

### 198. Alias-copy leak regression — `p146_script_95_alias_copy_leak` *(CLOSED 2026-05-01)*

**Status:** Closed in commit `30b01ce`.  Two coordinated fixes — one
in scope analysis, one in native codegen — both centred on unwrapping
the parser's `Value::Span` wrapper before pattern-matching the RHS of
`Set`.

**Test:** `tests/leak.rs::p146_script_95_alias_copy_leak` (passes on
the current branch; full `cargo test --release --test leak` reports
13/13).

#### Root cause

`scopes.rs::scan_set` and `generation/dispatch.rs::output_set` both
pattern-matched `Value::Call(...)` and `Value::Var(...)` directly,
without unwrapping the `Value::Span` wrapper that the parser puts
around most operators.  When the RHS of `ac_copy = ac_identity(ac_orig)`
arrived as `Span(Call(n_ac_identity, ...))`:

- `scopes.rs::scan_set` failed to clear `v`'s deps via
  `make_independent`, so `get_free_vars` skipped `OpFreeRef` emission
  for `ac_copy` → Database leaked (interpreter-mode P198 panic).
- `dispatch.rs::output_set` fell through to plain assignment instead
  of the `OpDatabase` + `OpCopyRecord` deep-copy → `ac_copy` aliased
  `ac_orig`'s store (native-mode runtime alias bug — same script,
  separate symptom, same root cause).

The wrapper was added by plan-07's Span IR walker work; the deep-copy
and deps-clearing paths missed the corresponding passthrough.

#### Shipped fix (2026-05-01)

Both functions now bind `unspanned_value` / `to_unspanned` once at the
top of the function and pattern-match against that.  The inner
deep-copy emit also moved from `stores` to `cell` for consistency with
the P199 ABI refactor that landed in the same window.

#### Original symptom (kept for context)

```
Database 3 not correctly freed (allocated by OpInitRef at pc=4788;
rerun with LOFT_LOG=alloc_free for the full trace)
```

The pc shifted between intermediate commits (`4788 → 4842 → 5037`) as
unrelated codegen changes landed, but the alloc-without-free shape was
unchanged until the Span unwrap was added.

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

### 213. `Type::Function` storage layout limit — full design for the proper fix

**Status:** parse-time diagnostics shipped on two surfaces (struct
fields 2026-05-04 commit 5b407d6; nested-closure captures 2026-05-04
follow-up).  These are the correct behaviour for the current release.
The layout-widening fix that lets fn-ref values actually persist
through every storage path is **wanted to stop the bug from
surfacing again** — we've already discovered it twice in one session
(P213 user struct fields + P215 nested-closure captures), and the
same root cause sits behind several open feature gaps in plan-15
(D3, D4, C4, C6) and a subset of plan-06's deferred work.  This
section is the design-of-record so the work can land cleanly when
prioritised.

#### Why we keep stumbling into this

The bug isn't one place — it's a layout decision (`element_size(Type::Function) = 4`)
threaded through every storage container.  Every container that
takes a `Type::Function` element silently truncates fn-ref values
from 16/20B down to 4B.  The corrupted bytes look like a random
d_nr at runtime, so failures present as "fn_call_ref: d_nr out of
range" / "Write to locked store" / `E0308` / `E0605` in different
contexts — none of which obviously points back at "your fn-ref slot
is too narrow."  Over time we'll keep filing P-issues against
*symptoms* of this single root cause unless we widen the layout.

**Why this matters for a future release:** capturing closures in
struct fields are the natural shape for several patterns loft will
eventually ship:

- **Async / IO event handlers** — a single struct holding multiple
  capturing callbacks (`on_message: fn(Message) -> void`,
  `on_error: fn(Error) -> void`, `on_close: fn() -> void`) where each
  callback closes over the connection's local state.
- **Server main loops tracking many signals** — a `Server` struct with
  fields like `on_request: fn(Request) -> Response`, `on_shutdown:
  fn() -> void`, `on_metric: fn(Metric) -> void`; each handler
  captures server-local state (config, counters, db handle).
- **Game main loops** — an `EventBus` struct or a per-entity
  `Behaviour` struct with capturing callbacks for collision,
  damage, level-up, AI tick.  The natural decomposition of a game
  loop puts handlers in fields keyed by signal name.
- **State machines** — transition tables stored as struct fields
  where each transition is a closure capturing the current state's
  context.

A programmer building any of these will reach for
`struct Handler { on_X: fn(...) -> ... }` and assign capturing
lambdas — that's the natural code to write.  Today they hit the
diagnostic; the workaround (top-level fns + manual context-passing)
becomes awkward at scale.  Forcing the pattern through plain
function pointers turns every real handler into a multi-arg fn that
re-derives context from arguments, defeating the point of having
local state.

The diagnostic is the right answer for the current release: it
keeps users out of the panic, points at the workaround, and stays
stable until the layout fix lands.  The proper fix is queued for a
future release rather than rushed in.

#### Why the diagnostic was shipped first

A struct field of `Type::Function` allocates 4 bytes (just the function id
`d_nr`) per the existing `element_size(Type::Function) = 4` in
`src/data.rs`.  A capturing closure value on the eval-stack is 16 bytes:
4B `d_nr` + 12B `DbRef` pointing at the closure record (which holds the
captured variables).  Writing 16B into a 4B field corrupted the
following 12 bytes of host-record state, manifesting as
"Write to locked store at rec=… fld=…" in interp and `E0308 mismatched
types` / `E0605 non-primitive cast` in native.

#### Surface inventory — every container that touches `Type::Function`

The layout limit appears wherever `element_size(Type::Function) = 4`
is consulted.  Today's storage container summary (✓ = works fully,
✕ = layout-limited / diagnostic shipped, ? = under-tested):

| Container | Non-capturing fn-ref | Capturing fn-ref | Status |
|---|---|---|---|
| Stack-frame variable (20B slot) | ✓ | ✓ | Works. `gen_fn_ref_value` pads to 16B; `OpPutFnRef` writes 20B; `fn_call_ref` reads d_nr at offset 0, closure DbRef at offset+4. |
| Struct field (`struct S { cb: fn(...) }`) | ✓ | ✕ (P213) | Diagnostic shipped — capturing closures rejected at parse time |
| Synthetic closure record's fn-typed field | ✓ | ✕ (P215, dup of P213) | Diagnostic shipped — captured fn-typed vars rejected at parse time |
| Tuple element (`(fn(...), …)`) | ✓ | ✕ | Existing test at `p4d_tuple_field_with_fn_ref` only covers non-capturing.  Capturing case panics symmetrically; no diagnostic yet. |
| Vector element (`vector<fn(...)>`) | ✕ partly (P214) | ✕ | Even non-capturing case rejects (P214); capturing case never worked.  Both share this layout root cause. |
| Hash/Sorted/Index value type | ? | ? | Untested.  Likely fails identically. |
| Reference-of-fn-typed (`Reference<fn(...)>`) | ? | ? | Probably nonsense / parse error already.  Worth confirming. |

**Scope of the current fix:** the co-located-closure-record approach
below targets the **struct field** and **synthetic closure record**
rows (which share a code path).  Tuple / vector / hash / sorted /
index / Reference rows are deferred follow-on work — they share the
same root cause but the co-located-record pattern needs per-container
extensions (e.g., a tuple element of fn-ref would need the closure
record allocated against the host record holding the tuple, with
the rec-id as part of the tuple element's bytes).  This staging
keeps the current fix's blast radius tight.

The diagnostics shipped today (P213 + P215 in the struct-field /
synthetic-closure-record paths) get *removed* when the struct-field
surface lands and are replaced with positive end-to-end tests
(matrix below).  Diagnostics (or current-behaviour-as-is) for
tuple/vector/hash rows stay in place until those rows' follow-on
plans land.

Pre-existing tests at `tests/issues.rs::p4d_fn_ref_as_struct_field`,
`p4d_fn_ref_field_default_init`, `p4d_fn_ref_field_bare_default`
exercise the non-capturing struct-field path; they must keep passing
through the layout change.  The capturing path was unsupported and
gets new positive tests with this fix.

#### The lifetime model is the same as vector-as-struct-field

A `vector<T>` struct field today stores a 4-byte rec-id pointing at a
vector-header record in the SAME Store as the host.  Pushed elements
become subsequent records in that Store.  Freeing the host struct
cascades to free all the vector's records via `copy_claims` /
`remove_claims` walking the host's `Parts::Vector` field
(`src/database/allocation.rs:937-1046`).  Cross-store moves
(`OpCopyRecord` from one Store to another) deep-copy the entire
nested record graph through the same cascade.  All this machinery
already exists.

A `Type::Function` struct field follows the identical pattern after
this fix: the host's field stores a rec-id pointing at a closure
record co-located in the host's Store.  The closure record holds the
captured-environment variables.  Free / copy cascade flows through
existing `Parts::Vector`-style walking; lifetime is exactly as long
as the host's; aliasing is impossible because field writes always
deep-copy.

The earlier "synthetic-struct with embedded 12B DbRef pointer"
approach was ruled out: an embedded DbRef can target a record in a
DIFFERENT Store than the host, decoupling the closure record's
lifetime from the host's.  Cross-store host moves leave the embedded
DbRef pointing at the original Store — dangling once the original
Store is freed.  Co-locating the closure record in the host's Store
removes that failure mode entirely.

#### Implementation plan — co-located closure record via OpAppendVector (revised 2026-05-04)

Two earlier framings of this plan are superseded:
- v1: "synthetic-struct + new Parts::FnRef variant" (inline 16B per
  field).  Cascaded too widely.  Reverted in commit 2938edf.
- v2: "v_set(w, host_ref) + OpDatabase retarget".  **Fatally
  broken** — a critical-pass review against the actual code surfaced
  that `OpDatabase` calls `database.clear(&db)` (`src/state/io.rs:637`)
  which calls `Store::init()` (`src/database/allocation.rs:306-316`).
  Setting `w` to `host_ref` would reinitialise host's Store, wiping
  the host record we just allocated.  The simple retarget is unsafe.

**v3 — OpAppendVector path.**  Use the existing
`OpAppendVector` op (`src/fill.rs:1661`, calls `vector_add` at
`src/database/structures.rs:149`, calls `vector_append` at
`src/vector.rs:97`).  Unlike `OpDatabase`, `vector_add` →
`vector_append` calls `store.claim(...)` directly without
`database.clear(...)`.  No store wipe.  This is the existing
"claim a child record in an existing store" primitive that
vector-as-struct-field already uses; we re-purpose it for fn-ref
struct fields.

**Critical-pass findings that shaped v3.**

| # | Finding (against code) | Impact |
|---|---|---|
| 1 | `OpDatabase` always calls `clear()` which wipes the target store | v2 was fatally broken — switched to OpAppendVector |
| 2 | No existing `OpRecOf`/`OpVectorFirstOrNull` primitive | Need one small new op (~15 lines) for the read path |
| 3 | Heap-captured pointers (text/Reference/vector) live in parent's Store; closure record in host's Store has dangling pointers if host outlives parent | Handled automatically at `copy_claims`/`remove_claims` time when host moves cross-store or is freed — but **only if the cascade actually walks**.  Plan inline-construction stays inside parent's scope, OR explicitly cross-Store-moves the host via `OpCopyRecord`-style cascade.  Pin with leak test. |
| 4 | `LinkedFieldGroup` is for "place fields contiguously", NOT "alias one loft attribute to N fields"; only used by `tuple_def` today | Drop LinkedFieldGroup; handle two database fields per loft attribute via custom codegen at set_field_check / get_val Type::Function arms (the loft attribute name "cb" doesn't need to map to a single database field — codegen knows to read both `cb__d_nr` and `cb__closure_rec`). |
| 5 | Parent-scope `OpFreeRef` suppression on `w` | Use existing `Function::set_skip_free(v)` at `src/scopes.rs:1192`; ~5 lines. |
| 6 | Closure record's `known_type` must be filled before host struct's field registration uses it | Mirror the existing tuple-as-field recurse-fill pattern at `src/typedef.rs:491-496`. |

**The architectural picture.**

The lambda's existing codegen (`src/parser/vectors.rs:710-735`)
allocates a closure record in a fresh dedicated Store via
`OpDatabase`.  We DO NOT touch that allocation.  Instead, at the
field-write site we use `OpAppendVector` to deep-copy the
parent-Store closure record into a freshly-claimed record in
host's Store as the first element of a vector held by the
`__closure_rec` field.  The parent-Store closure record then gets
freed by the existing parent-scope cleanup; the host owns its own
copy.

This is structurally the same pattern that `vector<T>` fields use
today — host's struct field stores a vector header, vector
elements live as records in the same Store.  We're treating the
fn-ref's closure half as a 1-element vector.

**Layout summary.**

```
struct Box { cb: fn(integer) -> integer }
                   ↓ database registration ↓
Parts::Struct of Box {
    cb__d_nr        : Parts::Int(4 bytes)         // function's d_nr
    cb__closure_rec : Parts::Vector(closure_kt)   // 4B vector header
                                                   // → element [0] is the
                                                   //   closure record in
                                                   //   host's Store; 0 = empty
                                                   //   (non-capturing case)
}
```

No LinkedFieldGroup needed.  The two database fields are tied
together purely by codegen knowing the naming convention.

The closure record's known_type is the existing
`synthesize_closure_record` schema — captured vars only; no
schema change.  It's allocated by the existing `OpDatabase` op
just retargeted at host's Store; freed automatically when the
host is freed via existing `copy_claims` / `remove_claims`
cascade walking the host's `Parts::Vector` field.

**Step 1 — Decide the storage Parts variant for the closure-rec field.**

Two options to evaluate at implementation time:

| Option | Description | Risk |
|---|---|---|
| (a) Reuse `Parts::Vector(closure_kt)` with single-element semantics | The host's `cb__closure_rec` field is registered as a vector pointing at exactly one record.  All `copy_claims` / `remove_claims` walking already exists. | Vector machinery assumes >0 length; need to confirm single-element case doesn't trip vector-specific opcodes (sequences, iteration).  Probably fine since the field is never iterated via loft-level vector ops. |
| (b) Add a new `Parts::ChildRec(u16)` variant | Mirror `Parts::Vector`'s cascade arms (~5 match sites in io/search/format/structures/allocation) but for "follow rec-id to a single child record". | Small new variant; cleaner semantics; ~30 lines of mechanical match-arm additions. |

Pick whichever matches existing patterns more cleanly during
implementation.  (a) is preferred for minimal change unless
single-element vector semantics actually break.

**Step 2 — Register the host's fn-ref field as two database fields.**

In `src/typedef.rs::fill_database` (around line 470, the
`Type::Function` arm):

```rust
Type::Function(_, _, _) => {
    // P213 — split fn-ref struct fields into two database fields:
    //   <attr>__d_nr        : 4B Parts::Int (function id)
    //   <attr>__closure_rec : 4B Parts::Vector / Parts::ChildRec
    //                         (rec-id of closure record co-located
    //                         in host's Store; 0 = null = non-capturing)
    let attr_name = data.attr_name(d_nr, a_nr);
    let closure_kt = /* known_type of the lambda's closure record */;
    database.field(s_type,
        &format!("{attr_name}__d_nr"),
        database.int(0, false));
    database.field(s_type,
        &format!("{attr_name}__closure_rec"),
        database.vector(closure_kt));  // or database.child_rec(closure_kt)
    register_fn_ref_field_group(database, s_type, attr_name);
    continue;
}
```

The closure record's known_type is the SAME type the lambda would
otherwise allocate.  Resolving it requires looking up the lambda's
closure_record d_nr — available via `data.def(lambda_d_nr).closure_record`
when the lambda's def is in scope, or stored on the host struct's
attribute as a side-data link.

**Step 3 — Field-write codegen — OpAppendVector path.**

In `src/parser/mod.rs::set_field_check`'s `Type::Function` arm:

```rust
Type::Function(_, _, _) => {
    let (alloc_steps, fn_ref) = split_fn_ref_for_field_write(&val_code);
    let Value::FnRef(d_nr, w, _) = fn_ref else { unreachable!() };
    let dnr_pos  = pos_val.clone();                  // cb__d_nr
    let crec_pos = pos_plus(pos_val.clone(), 4);     // cb__closure_rec

    let mut ops = Vec::new();

    // 1. Lambda's existing alloc_steps run as today — closure record
    //    is allocated in a parent-scope Store via OpDatabase.  We do
    //    NOT retarget it (v2 idea was broken — OpDatabase wipes the
    //    target store via database.clear()).
    ops.extend(alloc_steps);

    // 2. Write d_nr (4B) at the host's __d_nr position.
    ops.push(self.cl("OpSetInt4",
        &[ref_code.clone(), dnr_pos, Value::Int(d_nr)]));

    if w != u16::MAX {
        // 3. Capturing case: append the parent-Store closure record
        //    as element [0] of the host's __closure_rec vector.
        //    OpAppendVector → vector_add → vector_append:
        //      a. Claims a fresh record in host's Store (no clear).
        //      b. Deep-copies bytes from var(w)'s record.
        //    The vector header at `crec_pos` is updated.
        let crec_field = self.cl("OpGetField",
            &[ref_code.clone(), crec_pos.clone(), closure_kt_value]);
        ops.push(self.cl("OpAppendVector",
            &[crec_field, Value::Var(w), closure_kt_value]));
    }
    // Non-capturing case: vector stays empty (default header value 0
    // from struct allocation), no op needed.

    v_block(ops, Type::Void, "fn_ref_field_set")
}
```

Everything downstream is unchanged: closure record schema (no d_nr
embedded, just captures), capture writes, fn-ref slot shape on
stack, fn_call_ref.

**Step 4 — Field-read codegen.**

In `src/parser/mod.rs::get_val`'s `Type::Function` arm:

```rust
Type::Function(_, _, _) => {
    let dnr_pos  = p.clone();
    let crec_pos = pos_plus(p.clone(), 4);
    let read_dnr  = self.cl("OpGetInt4",
        &[code.clone(), dnr_pos]);
    // New op OpVectorFirstOrNull: reads vector header at crec_pos.
    // If empty (length 0): returns null DbRef sentinel which
    // fn_call_ref handles correctly (state/mod.rs:329).
    // If non-empty: returns DbRef of element [0] = the closure
    // record in host's Store.
    let read_clos = self.cl("OpVectorFirstOrNull",
        &[code.clone(), crec_pos]);
    v_block(vec![read_dnr, read_clos], tp.clone(), "fn_ref_field_read")
}
```

Result: 20B on stack (8B d_nr + 12B DbRef) — the fn-ref slot shape
`fn_call_ref` consumes unchanged.

**Step 4b — New op `OpVectorFirstOrNull`.**

Add to `src/fill.rs` (~15 lines):

```rust
fn vector_first_or_null(s: &mut State) {
    let v_fld = *s.code::<u16>();
    let v_host = *s.get_stack::<DbRef>();
    let store = s.database.store(&v_host);
    let vec_rec = store.get_u32_raw(v_host.rec, v_host.pos + u32::from(v_fld));
    let result = if vec_rec == 0 {
        DbRef { store_nr: u16::MAX, rec: 0, pos: 0 }
    } else {
        let length = store.get_u32_raw(vec_rec, 4);
        if length == 0 {
            DbRef { store_nr: u16::MAX, rec: 0, pos: 0 }
        } else {
            DbRef { store_nr: v_host.store_nr, rec: vec_rec, pos: 8 }
        }
    };
    s.put_stack(result);
}
```

And entry in `default/01_code.loft`:

```
fn OpVectorFirstOrNull(host: reference, fld: const u16) -> reference;
```

**Step 5 — Suppress parent-scope `OpFreeRef` on `w`.**

The lambda's `alloc_steps` allocate the closure record in parent's
Store and bind it to `w`.  After step 3's OpAppendVector deep-copies
the record into host's Store, parent's record is REDUNDANT but
still owned by parent's scope.  Today's `get_free_vars` would emit
`OpFreeRef(w)` at parent scope exit — that's CORRECT behaviour
because parent owns its allocation.

Note: this differs from v2, where we needed to suppress the free.
With v3 (OpAppendVector deep-copies), the parent's closure record
is independent of the host's; freeing it normally is fine.

So step 5 is **no-op** — existing dep tracking and `OpFreeRef`
emission for `w` are correct.

**Step 6 — Default-init.**

`to_default(Type::Function)` produces `Value::FnRef(0, u16::MAX, …)`
which step 3's `w == u16::MAX` non-capturing branch handles
correctly: writes 0 to `__d_nr` and 0 to `__closure_rec`.

`tests/issues.rs::p4d_fn_ref_field_default_init` and
`p4d_fn_ref_field_bare_default` cover this path; both must stay
green after the fix.

**Step 7 — Free / copy cascade — automatic via the chosen Parts variant.**

If step 1 chose `Parts::Vector` reuse: zero new code — existing
copy_claims / remove_claims arms walk it already.

If step 1 chose `Parts::ChildRec`: ~5 match-arm additions across
`src/database/allocation.rs`, `io.rs`, `search.rs`, `format.rs`,
`structures.rs` mirroring the Parts::Vector cascade.

**Step 8 — Native codegen mirror.**

In `src/generation/`:
- Field-write template emits the retarget (`v_set` of `w` to the
  host's native `(u32, DbRef)` shape) + native equivalents of
  `OpDatabase` and the capture writes.  Existing native helpers
  (`stores.claim`, `store.set_u32_raw`) compose into this.
- Field-read template emits `stores.store(&host).get_u32(...)` for
  d_nr and `stores.get_ref(&host, ...)` for the closure DbRef.

Run `cargo test --release --test wrap` and `cargo test --release
--test issues p4d_fn_ref` after the native-side change to catch
silent layout breaks.

**Step 9 — Remove the P213/P215 diagnostics + flip tests positive.**

Once steps 1-8 land:
- Delete `src/parser/mod.rs::set_field_check`'s P213 diagnostic
  (around line 2284-2293).
- Delete `src/parser/control.rs::try_fn_ref_call`'s P215 diagnostic
  (around line 3042-3093).
- Delete `src/parser/mod.rs::capturing_fn_ref` helper (no longer
  needed).
- Delete `tests/parse_errors.rs::p213_capturing_closure_in_struct_field_rejected`
  and `p215_captured_fn_in_nested_closure_rejected`.
- Add positive end-to-end tests in `tests/issues.rs` (see § Test
  matrix below).

**Out-of-scope at this stage (deferred to follow-on plans):**
- Non-inline assignment patterns: `f = fn(...){...}; b = Box { cb: f };`
  or `b = Box { cb: get_callback() };`.  These need either a
  diagnostic ("only inline lambdas can be stored in struct fields
  for now") or an OpCopyRecord-based deep-copy fallback.  The
  diagnostic stays in place for these specific shapes; the
  inline-only case is what this plan delivers.
- Field reassignment after host already exists: `b.cb = fn(...);`.
  Same constraint as non-inline assignment.
- Tuple / vector / hash element of fn-ref — same retargeting trick
  applies but per-container; deferred follow-on plans.

#### Existing tests to keep green

| Test | Path | Expected after fix |
|---|---|---|
| `p4d_fn_ref_as_struct_field` | `tests/issues.rs:1919` | Still passes — non-capturing fns write null sentinel to offset+4 |
| `p4d_tuple_field_with_fn_ref` | `tests/issues.rs:1985` | Still passes — same path, tuple element |
| `p4d_fn_ref_field_default_init` | `tests/issues.rs:2123` | Still passes — default `Holder { n: 7 }` writes null sentinel |
| `p4d_fn_ref_field_bare_default` | `tests/issues.rs:2138` | Still passes — `Holder {}` writes null sentinel |
| `p213_capturing_closure_in_struct_field_rejected` | `tests/parse_errors.rs` | **Removed** — capturing case now compiles instead of erroring |
| `p213_noncapturing_closure_in_struct_field_works` | `tests/parse_errors.rs` | Still passes |
| `p215_captured_fn_in_nested_closure_rejected` | `tests/parse_errors.rs` | **Removed** — nested-closure case now compiles instead of erroring |

#### Test matrix to pin the fix

When the layout fix lands, every cell here ships as a positive
end-to-end test (interp + native cross-mode parity via the
`cross_mode!` harness).  Cells are intentionally exhaustive — the
layout limit threads through enough container × capture-type
combinations that selective coverage will leave silent corruption
in production code we haven't yet written.

| Test | Container | Capture type | Pattern |
|---|---|---|---|
| `p213_struct_field_basic_int` | struct | `integer` | `struct S { cb: fn(int) -> int }; n = 5; s = S { cb: fn(x) { x+n } }; s.cb(10) == 15` |
| `p213_struct_field_text` | struct | `text` | capture `s = "tag"`, callback returns `"{s}: {x}"` |
| `p213_struct_field_reference` | struct | `Reference<S>` | capture a struct ref, callback returns a field of it |
| `p213_struct_field_tuple_capture` | struct | `(int, int)` | capture a tuple, callback uses both elements |
| `p213_struct_field_multi_capture` | struct | mixed `int + text` | two captures, disjoint types |
| `p213_struct_field_reassignment` | struct | any | `s.cb = new_lambda` — old closure record freed, new one copied; mirrors `Type::Reference` reassignment |
| `p213_struct_field_default_init` | struct | none (default) | `S { other: …}` with `cb` defaulted to null fn-ref; calling `s.cb(10)` should diagnose at parse time (calling a null fn-ref is undefined) |
| `p213_struct_field_aliasing` | struct | any | `s2 = S { cb: s1.cb };` — verify s2 has its own closure record (no shared mutation) |
| `p213_struct_field_freed_with_struct` | struct | any | leak test (mirrors `tests/leak.rs`): allocate, drop, assert closure record freed |
| `p215_nested_noncapturing` | closure record | `fn(int) -> int` (non-capturing inner) | `outer = fn(y) { inner(y) + 1 }` with `inner = fn(x) { x*2 }` at `outer`'s call site |
| `p215_nested_capturing` | closure record | `fn(int) -> int` (capturing inner) | `inner` itself captures from outer-outer scope |
| `p215_three_level_nesting` | closure record | nested closures | `f1 = fn() { f2 = fn() { f3 = fn() { … } } }` — two layers of synthetic records |
| `p4d_tuple_field_capturing_fn_int` | tuple | `int` | `(fn(int) -> int, integer)` where the fn captures |
| `p4d_tuple_field_capturing_fn_text` | tuple | `text` | text capture in a tuple-element fn |
| `p214_vector_capturing_fn_homogeneous` | vector | shared capture shape | `vector<fn(int) -> int>` of capturing lambdas with the same captured types |
| `p214_vector_noncapturing_lift` | vector | none | the existing P214 case (non-capturing in vector) — currently rejects, should pass after layout fix |
| `p213_hash_value_fn_typed` | hash | `int` | `hash<text → fn(int) -> int>` registry |
| `p213_sorted_value_fn_typed` | sorted | `int` | sorted-by-key with fn-typed value |
| `p213_index_value_fn_typed` | index | `int` | index of `(key, fn)` pairs |
| `p213_fn_field_passes_to_other_fn` | struct → fn arg | any | `do_thing(s.cb, x)` — fn-ref read from field, passed as fn-typed arg |
| `p213_fn_field_returns_from_fn` | struct → fn ret | any | `fn extract(s: S) -> fn(int) -> int { s.cb }` — fn-ref read from field, returned |
| `p213_fn_field_par_worker` | struct → par | any | `for s in items par(r = s.cb(x), 4) { … }` — captured fn-ref called inside parallel worker |

Cross-mode parity: every cell asserted via `cross_mode!` so interp
and native produce byte-identical stdout.  Leak tests (`*_freed_*`)
also assert via the `tests/leak.rs` framework that no store
allocations remain after `test()` returns.

Existing tests that must keep passing through the cascade:

| Test | Path | Why it matters |
|---|---|---|
| `p4d_fn_ref_as_struct_field` | `tests/issues.rs:1919` | Non-capturing struct field — should work after fix; the comment "closure half is intentionally null" updates to "closure half is null sentinel for non-capturing fn-refs" |
| `p4d_tuple_field_with_fn_ref` | `tests/issues.rs:1985` | Non-capturing tuple element — same |
| `p4d_fn_ref_field_default_init` | `tests/issues.rs:2123` | Default-init fn-ref field — null sentinel still correct |
| `p4d_fn_ref_field_bare_default` | `tests/issues.rs:2138` | Default-init bare struct — same |
| All `tests/scripts/*.loft` using fn-refs in any container | `tests/scripts/` | Run via `cargo test --test wrap` — layout cascade may surface silent breaks |
| Coroutines yielding fn-typed values (plan-16 Y5) | `tests/coroutine_matrix.rs` (when wired) | Yielded fn-ref must round-trip through the state-machine lowering with the new width |

Diagnostic-only regression tests to remove when the fix lands:

- `tests/parse_errors.rs::p213_capturing_closure_in_struct_field_rejected`
- `tests/parse_errors.rs::p213_noncapturing_closure_in_struct_field_works` (rename to positive test)
- `tests/parse_errors.rs::p215_captured_fn_in_nested_closure_rejected`

#### Risk register

| Risk | Mitigation |
|---|---|
| Free cycle out of sync — closure record not freed when host is freed, or freed twice | `Parts::Vector(closure_kt)` is already on the cascade path of `copy_claims` / `remove_claims` (`src/database/allocation.rs:937-1046`).  No new wiring needed.  Pin with leak regression test (`p213_struct_field_freed_with_struct`) using `tests/leak.rs` style. |
| Heap-captured pointers (text / Reference / vector) inside the closure record point at parent's Store after the deep-copy lands | `copy_claims` already deep-copies these field types when their parent record is moved.  At deep-copy time (OpAppendVector → vector_add → byte copy), the captured-pointer bytes are copied verbatim — POINTERS still point at parent.  When parent's Store survives long enough for the host to copy out (e.g., host returns from parent fn → `copy_record` cascade fires → captures get deep-copied), this is fine.  When host stays in parent's scope and uses captures while parent's vars are alive, fine.  **DANGER ZONE:** host with heap-capturing closures persists past parent's Store's death without ever crossing-store-copying.  Pin with `p213_struct_field_text_capture_escapes_via_return` test that verifies cross-store deep-copy on return. |
| Allocation overhead — closure record exists in BOTH parent's Store AND host's Store after the field-write | Parent's closure record is freed by parent's existing `OpFreeRef(w)` at scope exit; this is correct because parent owns its allocation.  The deep-copy in the host's Store is independent.  Bounded overhead: 1 record per fn-ref field per Box construction.  Future optimisation: skip the parent allocation entirely for inline-lambda-as-field-value cases (host's Store known at parse time); deferred. |
| `OpDatabase` clears the target store via `database.clear()` — proven lethal for retarget approach | Use `OpAppendVector` instead (does NOT call `clear()`; just `store.claim()` + byte copy).  Plan revised; v2 retarget abandoned. |
| `OpVectorFirstOrNull` is new infrastructure | ~15 lines in `src/fill.rs` + 1 line in `default/01_code.loft`.  Mirrors existing vector-element-read patterns.  Includes both backends (interp + native via `codegen_runtime`). |
| Aliasing — same lambda literal stored into two struct fields | Each field-write does its own OpAppendVector deep-copy.  No aliasing because each host has its own closure record co-located in its own Store.  Matches loft's existing single-owner model. |
| Native codegen wrinkles around the `(u32, DbRef)` fn-ref tuple representation | Native side mirrors interp via the runtime helpers.  `OpAppendVector` already has a native code path (used by vector struct fields).  `OpVectorFirstOrNull` needs a native helper; ~10 lines in `codegen_runtime.rs`. |
| Default-init fn-ref field (struct allocated without explicit `cb: …`) | `to_default(Type::Function)` produces `Value::FnRef(0, u16::MAX, …)` — step 3's `w == u16::MAX` branch writes 0 to `__d_nr` and skips OpAppendVector (vector header stays at default 0 = empty).  Field-read returns null DbRef sentinel; calling such a fn-ref is undefined (expected for default-init).  `tests/issues.rs::p4d_fn_ref_field_default_init` pins this. |
| Closure record's `known_type` resolution at typedef.rs:470 (Type::Function arm) | Mirror tuple-as-field's recurse-into-fill pattern at `typedef.rs:491-496`.  Look up the lambda's `closure_record` via the loft attribute's metadata; ~5 lines added. |
| WASM codegen path | The WASM target piggybacks on the same Op stream; no WASM-specific changes expected.  Validate by running `tests/html_wasm.rs` after the fix. |
| Tuple/vector-of-fn-ref tests (`p4d_tuple_field_with_fn_ref`, vector-of-fn-ref) might pick up changes unintentionally | Plan explicitly leaves `element_size(Type::Function) = 4` and the tuple/vector-of-fn-ref paths unchanged.  Tuple/vector containers keep their existing 4B-d_nr-only behaviour and current diagnostics.  Only the struct-field path uses the new two-database-field layout. |

#### Sequencing

Not gating the current release.  Co-schedule with whichever of these
lands first — they're the natural consumers and the work is cheaper
done together than retrofitted later:

- The `server` library (`doc/claude/WEB_SERVER_LIB.md`) — handler
  registries are the canonical use case.
- The `game_client` / OpenGL game-loop work that registers per-entity
  behaviours via callback fields.
- Plan-15 phase 03 (closure-DbRef leak fix) — overlaps with this
  plan's heap-capture lifetime tests; co-schedule.

Reasonable internal order once started:
1. Step 4b (new `OpVectorFirstOrNull` op) — smallest standalone unit;
   ship + test in isolation against existing vector code.
2. Step 2 (typedef.rs two-database-fields per attribute) — verify
   that allocating a host struct with a fn-ref field still compiles
   and existing non-capturing tests still pass (vector header reads
   as 0 → null DbRef → fn_call_ref's null path).
3. Step 3 (set_field_check OpAppendVector emit) — basic capturing
   case works.
4. Step 4 (get_val OpVectorFirstOrNull emit) — read path mirrors.
5. Reproducer at /tmp/p213.loft passes.
6. Step 8 (native codegen mirror).
7. Step 9 (remove diagnostics + flip tests positive).
8. Pre-flight a server-handler-registry mini-spike to confirm the
   design holds at real-program scale.

#### Out-of-scope

- Allowing closures to capture *by reference* (loft's closure model is
  copy-at-definition, per [DESIGN_DECISIONS.md § C38](DESIGN_DECISIONS.md#c38--closure-capture-is-copy-at-definition)).
  Field-stored closures stay copy-at-definition.
- Heterogeneous `vector<fn(...) -> ...>` of capturing closures with
  different capture shapes (per the loft-write skill restriction).
  The layout widening makes it possible in principle, but the type
  system still requires homogeneous element types in the vector — a
  separate plan-15 question.

#### Estimated effort

Revised 2026-05-04 (v3 — OpAppendVector path after critical-pass
findings against v2): 1-2 focused sessions.

- Step 1 (no decision needed — `Parts::Vector(closure_kt)` reused).
- Step 2 (typedef.rs split-attribute registration with two database
  fields per loft attribute, recurse-fill closure_kt) — ~30 lines.
- Step 3 (set_field_check codegen — alloc_steps + OpSetInt4 +
  OpAppendVector) — ~30 lines parser + helpers.
- Step 4 (get_val codegen — OpGetInt4 + OpVectorFirstOrNull) — ~15 lines.
- Step 4b (new op `OpVectorFirstOrNull`) — ~15 lines fill.rs +
  1 line 01_code.loft.
- Step 5 (default-init verification) — no new code.
- Step 6 (parent-scope OpFreeRef for `w`) — no change; existing
  behaviour is correct under the deep-copy model.
- Step 7 (free / copy cascade) — no new code (Parts::Vector
  already participates in copy_claims/remove_claims).
- Step 8 (native codegen mirror) — ~40 lines following existing
  vector-element-allocation native templates.  Includes
  `OpVectorFirstOrNull`'s native runtime helper.
- Step 9 (remove diagnostics + flip tests positive) — ~30 lines net
  (delete diagnostics + helpers, add 4-6 positive tests).

Run full suite + `tests/leak.rs`-style heap-capture leak test
after each step.

**Lesson from the 2026-05-04 spike:** the previous design widened
storage to 20B inline per fn-ref field, which cascaded into every
container that touches `Type::Function` storage (vectors, tuples,
hashes).  The co-located-record approach avoids that cascade by
keeping the field size unchanged at 4B (the rec-id pointer) and
moving the closure data to a child record.  Lifetime stays correct
by construction because the child record lives in the host's Store.

#### Why prioritise this even though no shipped feature gates on it

We've discovered this layout limit twice in one session (P213 +
P215, both folded here).  The surface inventory above shows several
other containers that haven't been carefully tested against capturing
fn-refs (hash, sorted, index, fn-typed Reference) — those will
produce more P-issues whenever a programmer reaches into them with
a capturing closure.  Each of those will look like a different bug
("array index out of bounds in hash::find", "stale DbRef in
sorted::insert", etc.) and require diagnosis from scratch.

Fixing the layout once removes the entire class.  The cost is a
focused 1-2 sessions; the alternative is paying diagnosis cost
every time the bug surfaces in a new container, plus shipping
diagnostics that have to be written and removed each time, plus
the risk that one of the silent-truncation cases (e.g. deep inside
a hash bucket walk) lands in production code before we notice.

This is the kind of root-cause fix that compounds: the same code
that lifts P213 + P215 also closes plan-15 D3/D4/C4/C6, partly
unblocks plan-06's deferred fn-typed worker patterns, and shrinks
the design surface we have to validate per release.

---

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
