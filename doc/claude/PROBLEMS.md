
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
- [🔴 Currently Open (fast index)](#-currently-open-fast-index)
- [Open Issues — Quick Reference](#open-issues--quick-reference)
- [Unimplemented Features](#unimplemented-features)
- [Interpreter Robustness](#interpreter-robustness)
- [Web Services Design Constraints](#web-services-design-constraints)
- [Graphics / WebGL](#graphics--webgl)

---

## 🔴 Currently Open (fast index)

**At-a-glance list of every open P-issue.**  Each row jumps to the
full entry in the Quick-Reference table below.  Keep this in sync
with the table — `tests/doc_hygiene.rs::problems_open_index_matches_quickref`
asserts a row appears here iff the table row's severity column
contains `(open)`.  Run `make problems` for the same list from
the command line.

| # | Severity | One-liner |
|---|---|---|
| [P229](#open-issues--quick-reference) (partial) | Low (Windows half open) | `parallel { … }` worker stack snapshot bug — Windows half still open after 2026-05-10 fix |

**Pattern**: the 5-bug bounded-generic cluster
(P252 + P239 + P240 + P243 + P241) closed 2026-05-11 via
substitution-time pattern extensions in
`parser/mod.rs::substitute_type_in_value`,
`parser/mod.rs::rewrite_generic_vector_writes`, and
`generation/emit.rs::{patch_hoisted_returns, infer_type}`.
P241 was the structurally hardest: vector-element write IR
is `OpNewRecord` + `OpCopyRecord` (Reference shape) for
parametric T at parse time, but needs `OpPreAllocVector` +
`OpSetInt` (or typed setter) for primitive T.  The shape
mismatch was bridged by a post-substitution IR walker that
detects the parametric triplet, rewrites it to the primitive
shape, AND patches the elm var's type back to `Reference(...)`
(the var holds a DbRef from `OpNewRecord` regardless of T's
concrete type, but `vars.substitute_type` had turned it into
the substituted primitive type).  **P255 closed (2026-05-12)**
extended the same triplet-rewrite arc to struct T (Reference):
keep OpCopyRecord but patch its `tp` arg + the surrounding
OpNewRecord/OpFinishRecord `parent_tp` args from the parametric
T's type-id to the concrete struct's type-ids, AND look through
`Type::Rewritten` wrappers in both `Type::is_equal` (so the LHS
type check passes) and `resolve_type_var` (so the wrapper does
not propagate into the bound T).  The struct-T fix completes
the bounded-generic cluster — every concrete element type now
goes through the substitution-time IR rewrite uniformly.
P254 closed (2026-05-12) — owner/permission/symlink/SUID
checks at cache lookup, mode-tightening at cache write, plus
a `LOFT_NATIVE_NO_CACHE=1` opt-out for paranoid users.  HMAC
+ secret was rejected as overkill for the documented threat
model (local-attacker only).  P256 closed (2026-05-12) — was
a stale test asserting the pre-plan-07-phase-4-step-4.8
silent OOB behaviour; updated to use only in-range byte
indices.

Open issues now: **P229b** (Windows multiplayer flake) +
**P266** (codegen routes ALL external-package native fns
through one crate (`loft_server` in viewer's case) instead
of dispatching by fn ownership; rustc E0425 — surfaced
2026-05-13 once P262/P263/P265 cleared).
**P262** closed 2026-05-13 (one-line `unspan()` in
`src/generation/calls.rs::user_fn_call_body`).
**P263** closed 2026-05-13 by renaming lib/web's loft-side
internal native-fn names to `ws_client_*_native` (matching
their existing `#native "n_ws_client_*"` annotations) so they
no longer collide with lib/server's `ws_*_native`.
**P264** closed 2026-05-13 by replacing `parse_string`'s
byte-by-byte `out.push(bytes[i] as char)` with a UTF-8
codepoint-length-aware `out.push_str(slice)` that preserves
multi-byte sequences verbatim.
**P265** closed 2026-05-13 by binding text-typed `_farg_N`
locals via a `&*` holder in the fn-ref dispatcher emit site
(`src/generation/emit.rs::output_call_ref`) so per-arm calls
pass `&str` regardless of whether the source expression
produced `Str`, `String`, or already-`&str`.
The plan-22 trio closed 2026-05-13:
- **P259** (Plan-22 Case C multi-instance crash) — OpIncRc +
  cascade-free chain.
- **P260** (closure captures struct → stale snapshot) — one-line
  fix in `synthesize_closure_record` (always-by-DbRef for
  Reference captures).
- **P261** (vector-field literal-assign appended instead of
  replaced) — added missing OpClearVector prepend in
  towards_set's vector-literal-replace branch.

**Surfaced 2026-05-13 by plan-35 phase 01:**
- **P262** — native codegen FAILED to wrap inline
  text-returning calls in `&*(…)` when the consumer expected
  `text`, producing un-compilable Rust like
  `f(n_returns_text(...))` with rustc E0308 ("expected `&str`,
  found `Str`").  PROBLEMS.md initially recorded this with
  the symptom direction reversed (claimed extra `&` wrap);
  the actual bug was a MISSING wrap.  **Closed (2026-05-13)**:
  `src/generation/calls.rs::user_fn_call_body`'s `needs_deref`
  check matched `Value::Call(d, _)` without unspanning the
  IR — the parser wraps inline call expressions in
  `Value::Span` for source-position tracking, so the bare
  matches!() pattern missed them.  One-line fix: bind
  `let v_unspanned = v.unspan();` first, match against that.
  Pinned by `tests/scripts/repro_p262.loft`.
- **P263** — `lib/web` declared loft-side internal functions
  with the SAME names as `lib/server` (`ws_recv_native`,
  `ws_message_native`, `ws_opcode_native`, `ws_send_native`,
  `ws_send_binary_native`, `ws_close_native`) but bound them
  to DIFFERENT `#native` impls (`n_ws_client_*` in lib/web
  vs `n_ws_*` in lib/server).  Codegen emits one
  `n_<loft_fn>_native` Rust function per loft declaration →
  rustc rejected the generated source with E0428.  Surfaced
  for any program that uses lib/server (lib/server's
  loft.toml depends on lib/web transitively).  **Closed
  (2026-05-13)**: renamed lib/web's loft-side names to
  `ws_client_*_native` (mirroring their existing `#native
  "n_ws_client_*"` annotations).  All call sites internal
  to lib/web/src/web.loft; no public-API breakage.  Native
  cdylibs (lib/web/native, lib/server/native) unchanged —
  the rename is loft-side only.

**Surfaced 2026-05-13 by P263 fix verification — closed same day:**
- **P265** — text-returning native fns called via fn-ref
  dispatch (`on_message: fn(text)`, similar callbacks) bound
  the call result to a `let _farg_0 = native_call(…);` local
  that was typed `Str`, then dispatched through a `match` to
  one or more `&str`-parameter callees with NO `&*` deref —
  rustc rejected with E0308 ("expected `&str`, found `Str`").
  Same root cause as P262 (Str / &str mismatch) but a
  DIFFERENT emit site: the fn-ref dispatcher in
  `src/generation/emit.rs::output_call_ref`, not the direct
  call-arg path P262 fixed in `src/generation/calls.rs`.
  **Closed (2026-05-13)**: bind text-typed `_farg_N` via a
  holder + `&*` deref so the per-arm calls receive a `&str`
  regardless of whether the source expression produced `Str`,
  `String`, or already-`&str`.  Condition `i <
  param_types.len() && matches!(param_types[i], Type::Text(_))`
  ensures the work-buffer arg (which sits beyond `param_types`
  and is `Type::RefVar(Type::Text)` typed) is unaffected.
  Pinned by `tests/scripts/repro_p265.loft` (cb: fn(text)
  dispatching `make_text() -> text` to a `consume(s: text)`
  callee).

**Surfaced 2026-05-13 by P265 fix verification:**
- **P266** — codegen emits external-package native-fn calls
  with the WRONG crate prefix.  Every call to a fn declared
  via `#native "n_…"` in any library is routed through
  `loft_server::n_…` even when the fn lives in
  `loft_web::n_…` (or any other native crate).  rustc
  rejects with E0425 ("cannot find function `n_<name>` in
  crate `loft_server`", with helpful "similarly named function"
  suggestions pointing at the wrong crate's contents).
  Reproducer: `use web; use server; fn main() { … }` under
  `--native` — the viewer trips this for `n_http_do`,
  `n_http_body`, `n_ws_connect`, `n_ws_client_send`,
  `n_ws_client_send_binary`, `n_ws_client_recv`,
  `n_ws_client_message`, `n_ws_client_opcode`,
  `n_ws_client_close`, `n_sleep_ms`, `n_pack_*`, `n_byte_at`
  — all owned by `loft_web/native`, all routed through
  `loft_server` → 16 E0425s.  Pre-existing — masked by P262 /
  P263 / P265 in any lib/web user before today.  Saved
  reproducer at `/tmp/p_followups/p266_wrong_crate_routing.sh`.
  Likely root cause in the codegen step that picks the
  `extern crate` prefix when emitting unsafe calls — appears
  to use the LAST loaded native crate name globally instead
  of looking up which crate owns each fn (presumably tracked
  via the `#native` annotation's source library).  **Workaround:**
  use `loft --interpret` for any program that pulls multiple
  native libraries.

**Surfaced 2026-05-13 by plan-37 phase 04a — closed same day:**
- **P264** — `json_parse` decoded JString payloads byte-by-byte:
  each input UTF-8 byte (e.g., `0xE2`, `0x86`, `0x92` for `→`)
  became a separate codepoint (U+00E2, U+0086, U+0092), each
  re-encoded as 2-byte UTF-8 (`c3 a2`, `c2 86`, `c2 92`).
  Net: 3 bytes in → 6 bytes out, displayed as `âââ` in the
  browser.  Direct string literals (`s = "→"`) round-tripped
  correctly; the bug was JSON-parser-specific.  **Closed
  (2026-05-13)**: `src/json.rs::parse_string` previously fell
  through to `out.push(bytes[i] as char)` for any non-escape,
  non-control byte — widening each byte to a separate
  codepoint.  Replaced with a UTF-8-aware path: `utf8_lead_len`
  reads the encoded length from the lead byte (1/2/3/4), then
  `out.push_str(std::str::from_utf8(&bytes[i..i+n]))` slurps
  the whole codepoint.  Safe because the input came from a
  `&str`, so the bytes are valid UTF-8 by construction.
  Pinned by `src/json.rs::tests::p264_multibyte_utf8_passthrough`
  exercising 2-byte (`café`), 3-byte (`→`), and 4-byte (`😀`)
  codepoints + a mixed-width string.  Viewer verification:
  `/tag/P259` now renders `gets \`P259\` → \`@P259\`` with
  the actual arrow character preserved.

**No high-severity bugs outside the generics + tuples cluster.**
Everything else from the older P-series (P198–P237 plus most of
P242–P249) closed during plan-09 / plan-14 work.  Phase 4 of
plan-07 shipped the typed-error infrastructure without
introducing new open P-issues.

---

## Open Issues — Quick Reference

| # | Issue | Severity | Workaround |
|---|-------|----------|------------|
| 264 | `json_parse` decoded JString payloads byte-by-byte — each input UTF-8 byte was widened to its own codepoint, then re-encoded as 2-byte UTF-8.  3-byte `→` became 6-byte `âââ`.  Surfaced 2026-05-13 by plan-37 phase 04a viewer rendering `index/tags.json` ref contexts containing `→`.  Direct text literals round-tripped fine; the bug was in the JSON parser's JString decode path.  **Closed (2026-05-13)**: `src/json.rs::parse_string` previously did `out.push(bytes[i] as char)` per byte; replaced with a UTF-8-aware path that reads the encoded length from the lead byte (`utf8_lead_len`) and `push_str`s the whole codepoint slice via `std::str::from_utf8`.  Safe because the input came from a `&str` — bytes are valid UTF-8 by construction.  Pinned by `src/json.rs::tests::p264_multibyte_utf8_passthrough` (2/3/4-byte codepoints + mixed-width).  Viewer verification: `/tag/P259` now renders the actual `→` character. | Medium (closed) | n/a — fix lands in `src/json.rs::parse_string` + new `utf8_lead_len` helper. |
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
| 235 | For-loop tuple destructure `for (a, b) in pairs { ... }` rejected with "Expect variable after for" — even WITHOUT par.  Surfaced 2026-05-07 during the ARC.md A7 actual-error survey on `par_tuple_destructure_in_for`.  **Non-par half closed (2026-05-07)** in `src/parser/collections.rs::parse_for` — parser now accepts `(name1, name2, …)` after `for`, synthesizes a temp loop var named `__destructure_t_<line>_<pos>`, defines each user-named binder as a proper variable typed as the matching tuple element, and prepends `Set(name_i, get_val(loop_var, offset_i))` ops to the body so each iteration unpacks the tuple before the user code runs.  Handles both direct `Type::Tuple([…])` (rare) and the common `Type::Reference(__tuple<…>)` shape (vector<(T1,T2)> iteration via P189b's element-access path).  **Par half closed (2026-05-09)** in `src/parser/collections.rs::parse_destructure_par_worker` — when `parse_parallel_for_loop` is called with `destructure_names` `Some`, the destructured names are defined in scope BEFORE worker parsing (mirroring the non-par destructure setup), then the worker call is parsed manually (capturing ALL args, not skipping the first like `parse_parallel_worker` does).  Second pass synthesizes a wrapper fn `__par_destructure_w_<line>_<pos>_<work>(t: tuple_type) -> ret { work(t.<i>, t.<j>, …) }` via `data.add_def + add_attribute + set_returned`; the wrapper's variable table is built fresh (`Function::new + add_variable + become_argument + defined`).  Each user arg that is `Var(destructure_var_nrs[i])` becomes a tuple element read at the matching position via `get_val`; non-destructure-var args pass through verbatim.  Par dispatch then calls the wrapper with the tuple loop element as its single per-iteration arg.  The wrapper supports non-positional arg ordering (`work(b, a)` correctly maps b → t.1 + a → t.0).  Pinned by the previously-ignored `tests/threading_chars::par_tuple_destructure_in_for` (un-ignored 2026-05-09) plus three new regression tests: `tests/issues.rs::p235_par_half_two_arity_int_int` + `_three_arity` + `_args_swapped`.  Threading-chars ignore count drops from 2 → 1 (heterogeneous-vec-of-fn / D11a row 8 stays as out-of-scope). | Low-Medium (closed) | n/a — fix lands in `src/parser/collections.rs` (parse_destructure_par_worker + destructure_names plumbing through parse_parallel_for_loop). |
| 236 | A function whose return type is a heap-owned reference (`Reference`, `Vector`, struct-enum) and whose body's tail expression is an `if/else` (or `match`) returning newly-constructed records corrupted the return value on `--native` (returned the typed null sentinel `DbRef { store_nr: u16::MAX, rec: 0, pos: 8 }` instead of the if/else's value).  Interpreter worked because `OpReturn(value=12)` reads from the eval-stack top (which the if/else branches populate); native generated `if cond { …; w } else { …; w }; OpFreeRef(w); return DbRef::null();` and the if/else's value was dropped on the floor.  Reproducer: `fn make_p(c: bool) -> P { if c { P{x:1, y:2} } else { P{x:3, y:4} } }` — interp printed `(1, 2)`, native printed `(null, null)`.  Verified pre-existing on `eebaaa4`.  Surfaced 2026-05-08 during ARC.md A7.1 (par tuple wide-return) when widening the parse_function gate to route pure-value tuple returns through `Reference(__tuple<…>)` exposed the same shape — `min_max(...) -> (integer, integer) { if cond { (a, b) } else { (b, a) } }` regressed `tests/docs/28-tuples.loft`.  **Closed (2026-05-08)** by three coordinated changes: (a) `parser/control.rs::unify_if_branches_work_refs` walks the body's tail If/Match (else-if chains lower to nested `Value::If`) and rewrites the false branch's terminal work-ref `Var` references via the new `substitute_work_ref` helper so both branches end with the SAME `Var(shared_w)`; gated to fire only when both terminal vars match `__ref_*` / `__rref_*` naming so user-named parameters are never renamed (regression caught when `gen_max<T: Ordered>(gen_x: T, gen_y: T)` accidentally returned `gen_x` for both branches).  (b) `scopes.rs::returned_var` recurses through `Value::If` and reports the shared var when both branches match, so `get_free_vars` skips OpFreeRef on it; `ref_return` then promotes it to a hidden caller arg as for single-branch reference returns.  (c) `scopes.rs::free_vars` catchall emits `Return(Var(ret_var))` instead of the legacy `Return(Null)` (which depended on the bytecode VM's eval-stack-top semantics — broken under native).  Same path closes the tuple variant: `parser/control.rs::rewrite_tail_tuple_to_synthetic_struct` recurses through If/Block/Insert with a SHARED work-ref via `rewrite_tail_tuple_with_work_ref`, so the synthetic-struct construction sequence reuses one DbRef across all branches.  Pinned by `tests/issues.rs::p236_struct_return_from_if_else_{true,false}` + `_else_if_chain` + `_param_returning_if_unaffected` (regression guard for the parameter-substitution gate) + `_tuple_return_from_if_else` + the previously-blocked `tests/threading_chars::par_tuple_return_{int_int,three_arity,nested}` canaries (un-ignored).  Side benefit: closes ARC.md A7.1 entirely. | High (closed) | n/a — fix lands in `src/parser/control.rs` (unify helper + tuple recursive rewrite + tail_has_tuple_leaf gate), `src/scopes.rs` (returned_var recursion + Return-with-ret-var emission), `src/parser/definitions.rs` (size-based tuple_rewrite gate widen), `src/parser/expressions.rs` (Reference(__tuple<...>) destructure arm). |
| 237 | A bounded-generic function returning a tuple where ANY element expression contains a bound-supplied operator call (e.g. `(x + x, 1)`, `(a, a + b)`, `(a == b, c)`) breaks both backends.  Reproducer: `fn pair_with_one<T: Addable>(x: T) -> (T, integer) { (x + x, 1) }` invoked with `pair_with_one(5)` — **interp SIGSEGV'd** (or returned garbage like `21509196218368`); **native rejected with rustc E0308** because the call-site monomorphisation substituted `T → i64` for the parameter type but the codegen for the body's operator call still referenced `t_1T_OpAdd(cell, var_x, var_x) -> DbRef`, giving a return-type mismatch (`expected i64, found DbRef`).  Distinct from P208 which was about text-return wrap.  Pre-flight Phase 02 of plan-17 surfaced this 2026-05-09; broadened during cell write-out same day when `(a, a + b)` (uniform-T tuple) failed identically to `(x + x, 1)` (mixed-T tuple) — the bug wasn't about the element type variation, it was about T's operator INSIDE a tuple constructor element.  **Closed (2026-05-09)** in `src/parser/mod.rs::substitute_type_in_value` by adding a `Value::Tuple` recursion arm — the function (which `re_resolve_call`s call-site references during monomorphisation) had handlers for Block / Set / Return / If / Loop / Insert / Iter / Span but not Tuple, so calls inside tuple-constructor elements stayed pointing at the generic `t_1T_*` stub instead of the concrete type's method.  Same pass added missing arms for `TuplePut`, `BreakWith`, `Yield`, `CallRef` (sibling Value variants the recursion had also forgotten).  Pinned by `tests/template_matrix::u3_b1a_addable_inline_pair_with_sum` (cross_mode harness, integer + float monomorphisations of `(x + x, 1)`). | High (closed) | n/a — fix lands in `src/parser/mod.rs::substitute_type_in_value` (Tuple/TuplePut/BreakWith/Yield/CallRef arms). |
| 238 | Bounded-generic function with `T: Equatable` (or any text-satisfying bound) returning a UNIFORM tuple of `T` (`-> (T, T)`) instantiated for `T = text` fails native compilation with rustc E0308 `expected &str, found String`.  Reproducer: `fn pair_t<T: Equatable>(a: T) -> (T, T) { (a, a) }` then `s = pair_t("hi");` — interp prints `hi|hi|hi|hi` correctly; native rejected with `let mut var_s: (String, String) = t_4text_pair_t(cell, "hi".to_string());` — call site passed `String` but the generated specialised fn's signature expected `&str`.  Distinct from P237 (mixed types) and P208 (text-return wrap).  Pre-flight Phase 02 of plan-17 surfaced this 2026-05-09.  Uniform-T `(T, T)` returns work for `T = integer` / `float` — the bug was text-specific.  **Closed (2026-05-09)** in two places: (1) `src/generation/dispatch.rs::output_call` now saves/clears `tuple_text_to_string` for the duration of argument processing — the flag set by the outer `let var_s: (String, String) = ...` assignment was leaking into `Value::Text` arg emission, causing `"hi"` → `"hi".to_string()` (mismatching the fn's `&str` signature).  (2) `src/generation/emit.rs` Value::Return arm now sets `tuple_text_to_string = true` when the function's `returned` is `Type::Tuple(elems with any Text)` and the body's return value is a `Value::Tuple` literal — required because the parser's tuple-of-text → synthetic-struct rewrite (Plan-14 phase 07 / P234) does NOT fire for generic monomorphisations (at parse time the tuple element type was a generic struct, not Text), so the return remained a plain `Value::Tuple` whose text elements needed a `.to_string()` wrap to fit the `(String, …)` slot.  Pinned by `tests/template_matrix::u2_b2_equatable_uniform_tuple_t` (cross_mode harness, both backends). | High (closed) | n/a — fix lands in `src/generation/dispatch.rs::output_call` (save/clear `tuple_text_to_string`) and `src/generation/emit.rs` Value::Return arm (set the flag for tuple-of-text returns). |
| 239 | A `for x in v { ... }` loop over `vector<T>` inside a generic function (any bound or no bound) crashed both backends.  Reproducer: `fn count<T>(v: vector<T>) -> integer { n = 0; for x in v { n = n + 1; } n }` invoked with `vector<integer>` — interp SIGSEGV on opcode dispatch; native rustc E0610 `i64.rec` (`i64` is a primitive type and therefore doesn't have fields) because the for-loop iter-termination check emitted `if !((var_x).rec != 0) { break }` — `var_x` was typed `i64` (the substituted T) but the IR still had the DbRef-shaped null-check.  Independent of bound — even bare `<T>` triggered it.  **Closed (2026-05-11):** the iter-termination check at `src/parser/collections.rs:1506-1514` emits `OpConvBoolFromRef(Var(loop_var))` for any loop variable typed `Reference(_, _)`, including `Reference(T_d_nr, …)` for generic-T element iteration.  When T monomorphises to a primitive, the substituted Var is now that primitive type but the IR still has `OpConvBoolFromRef` — interp treats `i64` as a `DbRef` (SIGSEGV) and native emits `i64.rec` (E0610).  Fix: extend `src/parser/mod.rs::substitute_type_in_value` to swap `OpConvBoolFromRef(Var(_))` to the matching primitive peer (`OpConvBoolFromInt` / `OpConvBoolFromText` / `OpConvBoolFromFloat` / `OpConvBoolFromSingle` / `OpConvBoolFromEnum`) when the substituted concrete type is a primitive.  Reference / Vector / struct-enum / tuple stay on `OpConvBoolFromRef` (the existing behaviour works for any DbRef-shaped loop variable).  Mirrors the P252 fix-family (extending substitution to handle the substituted-type's natural shape; same neighbourhood, different op).  Pinned by `tests/issues.rs::p239_for_loop_over_generic_vector` (interp + native both green). | High (closed) | n/a — fix lands in `src/parser/mod.rs::substitute_type_in_value` |
| 240 | A bounded-generic function whose body computes TWO OR MORE bound-supplied operator results into local variables, then returns them in a tuple constructor, produced wrong values under one backend in a way that depended on whether the body had intervening side-effects (e.g., a `println!`).  Reproducer: `fn classify<T: Ordered>(a: T, b: T) -> (integer, integer) { lt = if a < b { 1 } else { 0 }; gt = if a > b { 1 } else { 0 }; (lt, gt) }` invoked with `classify(3, 5)` — interp returned `(0, 0)`, expected `(1, 0)`; adding a `println` in the body flipped which backend was wrong.  **Closed (2026-05-11):** two compounding root causes both fixed.  **(a) Interp slot aliasing**: `src/variables/intervals.rs::compute_intervals` had no `Value::Tuple` arm — `Return(Tuple([Var(a), Var(b)]))` recursion fell through silently, the operand vars' last_use stayed at the default value, and the slot allocator considered them dead and aliased their slots.  When `a` and `b` were both written in the same scope, the second write clobbered the first; the tuple read returned `(b_value, b_value)`.  Fix: add `Value::Tuple(elems)`, `Value::TupleGet(_, _)`, and `Value::TuplePut(_, _, _)` arms that recurse into each element so reads update the operand vars' last_use correctly.  **(b) Native hoisted-return guard miss**: `scopes::free_vars` hoists the tuple value as a separate statement before the `OpFreeText(work)` cleanup, leaving `[Set(lt), Set(gt), Call(println), Tuple, OpFreeText, Return(Null)]`.  `src/generation/emit.rs::patch_hoisted_returns` already rewrites this to `Return(Tuple)` — but only when the block result type matches a small allow-list (Void / Never / `t_*` text-stub / `__ret_N` text temp).  T-stubs returning a tuple weren't in the list, so native emitted the tuple as a discarded statement and fell through to a hardcoded `return (0, 0)`.  Fix: add `is_t_stub_tuple_body` to the guard so tuple-returning T-stubs also run the patch.  Without (a) the no-side-effects shape was wrong on interp; without (b) the with-println shape was wrong on native.  Both shapes now correct on both backends.  Pinned by `tests/issues.rs::p240_bounded_generic_two_operator_tuple_return`. | High (closed) | n/a — fix lands in `src/variables/intervals.rs` (interp slot aliasing) + `src/generation/emit.rs` (native hoisted-return guard). |
| 241 | Building or pushing into `vector<T>` inside a generic function (any bound or no bound) crashes both backends.  Reproducer: `fn singleton<T>(x: T) -> vector<T> { out: vector<T> = []; out += [x]; out }` invoked with `singleton(42)` — interp panics with `index out of bounds: the len is 3 but the index is 42` at `src/database/allocation.rs:347`; native rejects with rustc E0308 because the generated code emits `OpCopyRecord(cell, var_x, var__elm_1, 9_i32)` but `var_x` is typed `i64` (the substituted T), giving `expected DbRef, found i64`.  Sibling of P239 (`for x in v` over `vector<T>`).  P239 is the consume side; P241 is the construct/push side.  **Closed (2026-05-11)** by a substitution-time triplet rewrite (Path B from the design analysis).  Implementation: `src/parser/mod.rs::rewrite_generic_vector_writes` walks the IR after both `substitute_type_in_value` AND `vars.substitute_type` have run, detects the parametric vector-element-write triplet `Set(_elm_, OpNewRecord(out, t_T, MAX)); Call(OpCopyRecord, [src, _elm_, t_T]); Call(OpFinishRecord, [out, _elm_, t_T, MAX])`, and when `concrete` is a primitive (Integer/Text/Float/Single/Boolean/Character/Enum/Function + narrow-int variants) rewrites it to the primitive shape (4 ops with `OpPreAllocVector` prefix for perf parity with concrete-T parse-time path) using `database.vector(database.db_type(concrete, data))` to resolve the concrete vector-element record type-id.  Critically, also patches `vars.set_type(elm_var, Type::Reference(content_def_nr, [out_var]))` because `vars.substitute_type` had turned `Reference(T, deps)` into the substituted primitive type (deps lost — primitives don't carry deps), but the elm var holds a DbRef from `OpNewRecord` regardless of T's concrete type; without the patch, codegen emits `VarInt`/`VarText`/etc. for an elm var that actually holds a ref, producing garbage at runtime.  Per-type setter dispatch lives in `Self::primitive_setter_call` and mirrors `parser/vectors.rs:1560-1599`.  Struct T (`Type::Reference`) is intentionally a no-op — the existing `OpCopyRecord` path is correct because the source IS a DbRef.  Pinned by `tests/issues.rs::p241_singleton_int` + `_text` + `_float` + `_bool` + `_in_if_branch` (interp + native both green).  Struct-T behavioural regression test deferred — capturing the returned `vector<T>` into a local variable hits a pre-existing, unrelated bug in `change_var`'s Vector→Vector branch (filed as P255).  Multi-element batched `OpPreAllocVector` (one prefix per N adjacent triplets) is a deferred follow-up — slice-1 emits per-triplet prefix (same per-push cost, just one extra prefix per multi-element push instead of per-batch).  Tuple-element T inside a generic vector and inline-struct-init (Value::Insert) shapes route through different parser branches and are out of scope; if a real reproducer surfaces, file as a new P-issue and extend the dispatch. | High (closed) | n/a — fix lands in `src/parser/mod.rs::rewrite_generic_vector_writes` + `Self::rewrite_vector_write_triplets` + `Self::match_vector_write_triplet` + `Self::is_primitive_vector_element_target` + `Self::primitive_setter_call`. |
| 242 | Format-string interpolation of a `T` variable inside a generic function body (`println!("val={x}")` where `x: T`) fails on both backends.  Reproducer: `fn show<T: Printable>(x: T) { println("val={x}") }` invoked with `show(7)` — interp printed `val=null` then panicked at `raw_vec/mod.rs:812`; native rejected with rustc E0308 because the codegen emitted `OpFormatDatabase(cell, &mut work, var_x, 9_i32, 0_i32)` but `var_x` was typed `i64` (the substituted T).  Sibling of P237 (operator-inside-tuple-element), P239 / P241 (vector ops): all share the root cause that the generic-fn codegen emits DbRef-shaped ops for T-typed values without substituting T's concrete type.  Surfaced 2026-05-09 during plan-17 phase 05 cell write-out.  **Closed (2026-05-09)** in `src/parser/collections.rs::append_data` by adding a `try_bound_to_text_call` helper that detects "format target is the current generic context's type variable AND the bound supplies a `to_text` method" — when so, the format value gets wrapped in a `Call(t_<len><tvname>_to_text, [format, work_buf])` IR node, and the format dispatch routes through `append_data_text` instead of the DbRef fallback.  At monomorphisation time `re_resolve_call` substitutes the stub with the concrete type's impl (same path explicit `x.to_text()` calls take).  The hidden `__work_1: RefVar(Text)` second arg (added by `parse_function`'s I9-text path) is supplied via a fresh work-text + `OpCreateStack` block, mirroring what `convert(text → &text)` would auto-generate.  Pinned by `tests/template_matrix::u8_b1p_printable_inline_t` (cross_mode harness, both backends). | High (closed) | n/a — fix lands in `src/parser/collections.rs` (try_bound_to_text_call helper + append_data Reference arm). |
| 243 | A bounded-generic function returning a tuple containing one or more `text` elements where one element is built by a bound-supplied method call (e.g. `fn show_pair<T: Printable>(x: T) -> (text, text) { (x.to_text(), "x") }`) silently returned empty strings on `--native`.  **Closed (2026-05-11):** three coordinated fixes that landed across today's session converged on this issue.  (1) P240's `is_t_stub_tuple_body` extension to `patch_hoisted_returns` made the tuple reach the Return position (was previously a discarded statement falling through to `return (String::new(), String::new())`).  (2) P238's `tuple_text_to_string` flag in `Value::Return` already set the wrap flag for `Tuple(text, …)` returns.  (3) **The remaining bug** was in `src/generation/emit.rs::infer_type` — it had no `Value::Span` arm.  The parser Span-wraps fault-prone calls (every `obj.method()` site) for source-position tracking, so the bound-method call inside the tuple was `Span(Call(…))`, not bare `Call(…)`.  `infer_type(Span(Call))` returned `None`, so the Tuple emit arm's `elem_is_text` check silently failed and the `.to_string()` wrap didn't fire — producing a `(Str, String)` tuple that rustc rejected with E0308.  Fix: add `Value::Span(b) => self.infer_type(&b.1)` to the match in `infer_type` so it transparently looks through position wrappers (same shape as the Span-aware patches elsewhere in the codebase).  Pinned by `tests/issues.rs::p243_bounded_generic_tuple_with_text_method_call`. | High (closed) | n/a — fix lands in `src/generation/emit.rs::infer_type` (Span-unwrap arm). |
| 244 | `--native` compilation of any program that links `lib/server` fails with rustc E0308 "expected `Str`, found `LoftStr`" on the generated wrappers for `n_ws_message_native` and `n_ws_event_payload_native` (and the latent siblings `n_tcp_method` / `n_tcp_path` / `n_tcp_body`).  Reproducer: `./target/release/loft lib/server/tests/server.loft` (default `--native` path).  Surfaced 2026-05-10 while wiring TTT v5's binary-WS bindings.  Root cause was in `src/generation/mod.rs::output_native_direct_call`: the wrapper for a `text`-returning native declared `-> Str` (per `rust_type` Context::Result mapping) but emitted the call expression unchanged, so the body returned the underlying extern's `LoftStr` directly — distinct type, no auto-conversion.  **Closed (2026-05-10)** by adding a `needs_text_wrap` branch parallel to the existing `needs_ret_cast` branch: captures the call's LoftStr return as a typed local (no annotation — sub-crates pull their own `loft_ffi`, multiple structurally-identical versions live in the dep graph), copies the bytes into `stores.scratch`, and hands back a `Str` borrowed from there (mirrors the P205 lifetime pattern).  Wrapper now reads `let _ls = unsafe { … }; let _bytes: Vec<u8> = …; stores.scratch.push(unsafe { String::from_utf8_unchecked(_bytes) }); Str::new(stores.scratch.last().unwrap())`.  Side benefit: latent siblings on `lib/web`'s `n_http_body` / `n_ws_client_message` now also work without further changes.  Pinned by `tests/codegen_emitter.rs::p244_text_native_wrapper_compiles_under_native` (invokes `loft lib/server/tests/server.loft` under default `--native` and asserts exit 0). | Medium (closed) | n/a — fix lands in `src/generation/mod.rs::output_native_direct_call` (`needs_text_wrap` branch). |
| 246 | File-scope `const NAME = expr;` panicked the parser with `index out of bounds: the len is 0 but the index is 65535` at `src/variables/mod.rs:938:37`.  Reproducer: a single line `const TYPE_DELTA = 2;` outside any function body.  Root cause: `parser/definitions.rs::parse_constant` only recognised the bare-name form (`NAME = value;`); the leading `const` keyword was treated as the constant's NAME, then the parser fell through to `expression()` (treating the real name as a fresh identifier), `parse_assign` allocated a stack slot at `u16::MAX` (the "no slot" sentinel — file-scope has no variable table), and `change_var_type` indexed the empty `self.variables` table at `u16::MAX` and panicked.  **Closed (2026-05-11)** by accepting the optional `const` keyword at file scope as a SYNONYM for the bare-name form (`const PI = 3.14;` ≡ `PI = 3.14;`).  The two forms now share the same code path, the same UPPER_CASE check, the same definition kind — `const` is purely an explicitness signal that matches the in-fn `const` syntax users already learned.  This also unblocked `lib/wall.loft:106`'s long-standing `const CORNER = [...]` declaration, which was de-facto broken until now.  Pinned by `tests/scripts/82-file-scope-const.loft` (regression guard for the keyword + bare-name forms + `pub const` + UPPER_CASE check + collision detection). | Low (closed) | n/a — fix lands in `src/parser/definitions.rs::parse_constant`. |
| 249 | Closure-typed tuple elements have a wrong-width layout AND a tuple-tail Return-wrap miscompile — `t = (lambda, 99); t.0(10)` crashed the interpreter with `fn_call_ref: d_nr=<garbage> out of range` at `src/state/mod.rs:319`, and `fn invoke(p: (fn(i)->i, text)) -> integer { p.0(7) }` failed native compilation with rustc E0308 `expected i64, found ()`.  Two layered root causes: (1) **20-byte fn-ref layout missed in five tuple codegen sites** — `src/state/codegen.rs::generate_var` Tuple arm + `Value::TupleGet` arm + `Value::TuplePut` arm + `emit_tuple_var_push_recursive` + `emit_tuple_var_pop_put` + `emit_tuple_put_ops` all bucketed `Type::Function` into the `Integer` arm and emitted `OpVarInt` / `OpPutInt` (8 B), but per P213 v4 / `variables::size`, fn-ref slots are 20 B (8 B i64 d_nr + 12 B closure DbRef).  Reading only 8 B left the closure half uninitialised; the call dispatch then read a garbage d_nr and crashed.  `src/data.rs::element_align`/`element_size` Function arm also returned 4 B / 4 B (legacy 16-byte layout assumption); both raised to 8 B align / 20 B size.  (2) **fn_call_tmp scope-cleanup wrapped by insert_free** — when a tuple-of-function param's TAIL CALL pattern (`p.0(7)`) is the function's return expression and the outer body block has its own pending frees (`OpFreeText` for a sibling work-text), `scopes::insert_free` saw the inner `fn_call_tmp` Block's last operator was `OpFreeRef(__fn_ref_tmp)` (added by inner-scope cleanup), inserted the outer frees BEFORE it, and wrapped THAT cleanup in `Return(...)` — the resulting native code returned `()` from a value-returning block.  **Closed (2026-05-11)** with two coordinated changes: (a) layout fix — split `Type::Function` from `Integer` in all six tuple codegen sites; emit `OpVarFnRef` + `stack.position += 4` on push (signature returns `text` = 16 B but runtime pushes 20) and `OpPutFnRef` + `stack.position -= 4` on pop, mirroring `set_var` / `generate_var`'s plain-Function paths.  Raise `element_align(Function)` to 8 and `element_size(Function)` to 20 in `src/data.rs`.  (b) ownership fix — `src/parser/operators.rs` postfix-call path now calls `self.vars.set_skip_free(fn_work)` on the `__fn_ref_tmp` temp, since its closure DbRef ALIASES the source's closure store (freeing it would double-free, and removes the trailing-OpFreeRef that triggered the insert_free Return-wrap miscompile).  Pinned by `tests/tuple_matrix.rs::e4_d{1_closure_local,1_closure_call,1_closure_swap,1_capture_survives,2_closure_arg}` (5/5 cells now green on both backends — captures via `fn(x: integer) -> integer { x + base }` form work too).  Phase 03 of plan-14 closes with this row.  Out-of-scope: T1.8a-style closure-tuple RETURN convention (still deferred). | Medium (closed) | n/a — fix lands in `src/data.rs` (element_align/size), `src/state/codegen.rs` (six tuple-arm splits), `src/parser/operators.rs` (set_skip_free on fn_ref_tmp). |
| 247 | Native codegen E0382 / E0597 when a nested tuple containing a non-Copy element (text / Reference) is read for format-string interpolation: `t = ((1, "a"), (2, "b")); print("{t.0.1}|{t.1.1}\n");` rejected with `use of moved value: 'var_t.0'`.  The format machinery extracts a `__ref_N: (i64, String) = var_t.0;` temporary that MOVES `var_t.0` (String isn't Copy), then a sibling read of `var_t.1` panicked because the move invalidated the parent tuple.  Surfaced 2026-05-11 by plan-14 phase 02's `e3_d1_text_inside` cell.  **Closed (2026-05-11)** with three coordinated changes: (1) `src/generation/dispatch.rs::output_set` — new `nested_tuple_clone` flag detects "destination is tuple-with-non-Copy-leaf AND source is `TupleGet(_, _)` of a tuple-typed parent"; emits `var_NAME.IDX.clone()` instead of the default move.  (2) `src/generation/emit.rs::Value::TupleGet` arm — for text-typed tuple elements where the source is a work-ref var (`__ref_N` prefix), emits `var___ref_N.IDX.clone()` (returns owned `String`) instead of `&var___ref_N.IDX` (borrow that escapes the enclosing Block scope and trips E0597).  (3) `src/generation/text.rs::format_text` wrap detection extended to `Value::Block` whose result type is `Type::Text(_)` so the `&*({block})` form forces temporary lifetime extension across the enclosing statement.  Pinned by `tests/tuple_matrix.rs::e3_d1_text_inside` (now green under `--ignored`).  Net effect: full plan-14 phase 02 matrix (5/5 e3 cells + 22/22 total) now passes. | Medium (closed) | n/a — fix lands in `src/generation/dispatch.rs` + `src/generation/emit.rs` + `src/generation/text.rs`. |
| 248 | Element-of-element assignment on nested tuples (`t.0.1 = 99` where `t: ((integer, integer), (integer, integer))`) rejected by both backends with `Not implemented operation = for type integer` at the parser level.  The parser's assignment LHS handling supported single-level `t.0 = x` (T1.x) but didn't recurse into the nested-tuple chain — operators.rs case-3 materialised `t.0.1` as `Block[Set(w0, TupleGet(t, 0)), TupleGet(w0, 1)]`; the assignment dispatcher saw a Block (not a TupleGet) and fell through to `compute_op_code` which emits the integer-= operator on a non-lvalue.  Surfaced 2026-05-11 by plan-14 phase 02's `e3_d1_elem_elem_assign` cell.  **Closed (2026-05-11)** with two coordinated changes: (1) `src/parser/expressions.rs` — new `extract_nested_tuple_lhs` helper recursively flattens the `Block[Set, TupleGet]` chain into `(root, [(work_var, idx)…], leaf_idx)`; `build_nested_tuple_assign` rewrites the IR as `Block[<existing reads>, TuplePut(deepest_w, leaf_idx, rhs), TuplePut(parent, idx, Var(w))…]` so the modification propagates back to root via single-level TuplePuts.  (2) `src/state/codegen.rs::TuplePut` — new `Type::Tuple(inner_elems)` element-type arm that calls the existing `emit_tuple_var_pop_put` recursive helper, so writing a tuple value into a tuple slot (the writeback step) emits per-leaf `OpPut*` ops at the correct offsets within the parent.  Both backends now accept arbitrary-depth nested LHS (`t.0.1.2 = …` works the same way).  Pinned by `tests/tuple_matrix.rs::e3_d1_elem_elem_assign`. | Medium (closed) | n/a — fix lands in `src/parser/expressions.rs` + `src/state/codegen.rs`. |
| 266 | Codegen emits external-package native-fn calls with the WRONG crate prefix.  Every `#native "n_…"`-declared fn is routed through `loft_server::n_…` even when the fn lives in `loft_web::n_…` (or any other native crate).  rustc rejects with E0425 "cannot find function `n_<name>` in crate `loft_server`" — with confusingly helpful "similarly named function" suggestions pointing at unrelated fns in the wrong crate.  Reproducer: `use web; use server; fn main() { web::ws_handler("…") }` under `--native` trips ~16 E0425s for `n_http_do`, `n_ws_client_*`, `n_sleep_ms`, `n_pack_*`, `n_byte_at` — all owned by `loft_web/native`, all routed through `loft_server`.  Pre-existing — masked by P262 / P263 / P265 in any lib/web user before 2026-05-13.  Reproducer saved to `/tmp/p_followups/p266_wrong_crate_routing.sh`.  Likely root cause: the codegen step that picks the `extern crate` prefix uses the LAST loaded native crate name globally instead of dispatching by fn ownership (presumably tracked via the `#native` annotation's source library).  Currently the only blocker between `--native` of the viewer and a fully working multi-language code-intelligence demo per plan-14. | High | Use `loft --interpret` for any program that pulls multiple native libraries. |
| 265 | Text-returning native fn called via fn-ref dispatch (`on_message: fn(text)`, similar callbacks) emitted `let _farg_0 = native_call(…);` (typed `Str`) then dispatched through `match` arms to multiple `&str`-parameter callees (`i_parse_error_push`, `n_print`, `n_println`, etc.) with NO `&*` deref — rustc E0308 ("expected `&str`, found `Str`").  Same root cause as P262 (Str / &str mismatch) but a DIFFERENT emit site: the fn-ref dispatcher in `src/generation/emit.rs::output_call_ref`, not the direct call-arg path P262 fixed in `src/generation/calls.rs`.  Reproducer: `use web; fn main() { println("ok"); }` under `--native` — lib/web's `pump()` registers `on_message: fn(text)` which triggers the dispatcher emit even when main doesn't call it.  Surfaced 2026-05-13 during P263 fix verification (once E0428 collisions removed); pre-existing.  **Closed (2026-05-13)**: bind text-typed `_farg_N` locals via a holder + `&*` deref so per-arm calls receive `&str` regardless of whether the source expression produced `Str`, `String`, or already-`&str`.  Condition `i < param_types.len() && matches!(param_types[i], Type::Text(_))` ensures the work-buffer arg (which sits beyond `param_types` and is `Type::RefVar(Type::Text)` typed) is unaffected.  Pinned by `tests/scripts/repro_p265.loft` — `cb: fn(text) = consume; cb(make_text())` exercises the dispatcher path.  Surfaced P266 (separate bug, wrong-crate routing) once the E0308s cleared. | High (closed) | n/a — fix lands in `src/generation/emit.rs::output_call_ref` (text-arg holder + `&*` deref bind). |
| 263 | `lib/web` declared loft-side internal functions with the SAME names as `lib/server` (`ws_recv_native`, `ws_message_native`, `ws_opcode_native`, `ws_send_native`, `ws_send_binary_native`, `ws_close_native`) but bound them to DIFFERENT `#native` impls (`n_ws_client_*` in lib/web vs `n_ws_*` in lib/server).  Codegen emits one `n_<loft_fn>_native` Rust function per loft declaration, so when both libs were pulled into a `--native` compile (lib/server depends on lib/web transitively per `lib/server/loft.toml [dependencies] web = ">=0.1"`), rustc rejected with E0428 ("the name `n_ws_recv_native` is defined multiple times").  Surfaced 2026-05-13 by plan-35 phase 01 (the viewer uses lib/server).  **Closed (2026-05-13)** by renaming lib/web's loft-side internal names to `ws_client_*_native` (mirroring their existing `#native "n_ws_client_*"` annotations).  All call sites are internal to `lib/web/src/web.loft`; no public-API breakage.  Native cdylibs (lib/web/native, lib/server/native) unchanged — the rename is loft-side only.  Verified: `--native --lib lib/ tools/viewer/src/main.loft` no longer reports E0428 (count: 0).  P265 (separate bug, surfaced by this fix) tracks the remaining E0308 in fn-ref dispatchers. | High (closed) | n/a — fix lands in `lib/web/src/web.loft` (rename of `ws_*_native` to `ws_client_*_native`). |
| 262 | Native codegen omitted the `&*` deref wrap when an inline text-returning user-function call was passed as an argument to another text-typed function call.  Generated Rust looked like `f(cell, n_returns_text(...))`; rustc rejected with E0308 ("expected `&str`, found `Str`") because text-returning user fns return the wrapper struct `Str` while parameters take `&str`.  Initial PROBLEMS.md row had the symptom direction reversed (claimed an EXTRA `&`); the actual bug was a MISSING wrap.  Reproducer: `fn greet(n: text) -> text { "hi {n}" } fn upper(s: text) -> text { s.to_uppercase() } fn main() { println(upper(greet("x"))); }`.  Surfaced 2026-05-13 by plan-35 phase 01 building the viewer's HTML body.  **Closed (2026-05-13)**: `src/generation/calls.rs::user_fn_call_body`'s `needs_deref` check matched `Value::Call(d, _)` against the spanned IR — the parser wraps inline call expressions in `Value::Span` for source-position tracking, so the bare matches!() pattern missed them and `needs_deref` stayed `false`.  One-line fix: bind `let v_unspanned = v.unspan();` first, match against that.  Mirrors the pattern at line 95 in the same function (`if let Value::Call(d_nr, args) = v.unspan()`).  Pinned by `tests/scripts/repro_p262.loft` (asserts `upper(greet("x")) == "HI X"` under both backends).  Viewer-emit verification: `target/release/loft --native-emit` on `tools/viewer/src/main.loft` now produces 22 `&*(n_…)` wraps where there were rustc errors before. | Medium (closed) | n/a — fix lands in `src/generation/calls.rs::user_fn_call_body`. |
| 261 | Vector-field assignment APPENDED instead of REPLACED on both backends, even outside closures.  Surfaced 2026-05-13 during P260 cell development.  Reproducer: `struct Bag { items: vector<integer> } fn main() { b = Bag { items: [1, 2, 3] }; b.items = [99, 100]; for x in b.items { print(" {x}"); } }` printed `1 2 3 99 100` (expected: `99 100`).  Same divergence on both interp + native — so the bug was in the parser's lowering of "assign to vector-typed field" rather than backend-specific codegen.  **Root cause:** `src/parser/expressions.rs` (towards_set's vector-field whole-replacement path, around line 936) handled three cases for `op == "="` against a vector field: (a) RHS is empty literal `[]` → emit OpClearVector; (b) RHS is a vector variable/expr → emit OpClearVector + OpAppendVector via Var-or-temp; (c) **RHS is a non-empty vector literal `[…]`** → fell through to the Insert-bypass with NO OpClearVector prefix, so the literal's element-construction ops (which build elements directly into the field's storage via OpNewRecord/OpFinishRecord) appended to the existing items instead of replacing them.  **Closed (2026-05-13)** by adding the missing `is_nonempty_literal` arm: prepend OpClearVector to the literal's statement list so the existing element-construction ops run on a fresh-empty field.  Pinned by `e_d3_struct_vector_assign_in_closure` in `tests/mut_closure_matrix.rs` (assertion now `sum == 199` for `[99,100]`, was buggy `sum == 205`). | Medium (closed) | n/a — fix lands in `src/parser/expressions.rs::towards_set` (vector-literal-replace branch). |
| 260 | Plan-22 / closure capture: closures that capture a struct saw a STALE SNAPSHOT of any non-scalar field (vector, nested struct, vector element).  Mutations from either side (closure body OR outer scope) silently no-opped against the closure's view.  Filed 2026-05-13 during plan-22 phase-04 follow-up testing.  **Root cause:** `synthesize_closure_record` in `src/parser/vectors.rs:1060-1069` only added the auto-Reference marker (`Type::Reference(d, vec![u16::MAX])`) to the closure record's attribute IF the capture was detected as mutated.  For ALL OTHER `Type::Reference` captures it left empty deps, which `typedef.rs::fill_database` (line 529-543) maps to inline-byte storage — i.e. the closure record held a deep copy of the struct's bytes captured at construction time.  Mutations from either side hit the inline copy; the original was never updated.  **Closed (2026-05-13)** with a one-line architectural fix: drop the `is_mutated` gate on the `Type::Reference` arm so storage is ALWAYS 12B Parts::DbRef pointing at the live original.  The auto-Reference marker is only consumed by `typedef.rs::fill_database` and only produced by `synthesize_closure_record`, so the change doesn't affect user-defined struct fields.  Six new `cross_mode!` cells in `tests/mut_closure_matrix.rs` lock the working behaviour: `e_d3_struct_vector_append_in_closure`, `e_d3_struct_vector_assign_in_closure`, `e_d3_struct_vector_element_assign_in_closure`, `e_d3_nested_struct_field_in_closure`, `e_d3_closure_reads_outer_struct_vector_mutation`, `e_d3_outer_reads_closure_vector_mutation`.  Surfaced P261 (orthogonal) — vector-field assignment APPENDS instead of REPLACES on both backends, which is consistent inside + outside closures (so P260's e_d3_struct_vector_assign_in_closure asserts the buggy-but-consistent `sum == 205` expectation that flips when P261 lands). | High (closed) | n/a — fix lands in `src/parser/vectors.rs::synthesize_closure_record` (drop is_mutated gate on Type::Reference arm). |
| 259 | Plan-22 phase 03 (Case C / factory pattern) — multi-instance interleaved-call pattern crashed with `index out of bounds: the len is 5 but the index is 6` at `src/database/allocation.rs:347` (`Stores::store(&r)`).  Reproducer: two factory closures constructed in sequence, then their calls interleaved.  **Root cause:** the cell's type after phase 02d-iii.a's flip is `Reference(__cell_*, vec![])` — heap-owned with empty deps.  At `make()`'s scope exit, `get_free_vars` saw the cell as heap-owned and emitted `OpFreeRef`, freeing the cell store while the closure record's auto-Reference attribute still held the cell's DbRef.  Single-factory survived by luck (freed store_nr isn't reused before the closure's calls finish); multi-factory crashed when the second `make()` reused the freed store_nr.  **Closed (2026-05-13)** with a coordinated four-commit fix that gives heap-owned cells the same ownership model as P213 ChildRecs — the closure record now actually OWNS the cells it captures: (1) added `OpIncRc(v1: reference)` opcode in `default/01_code.loft` that bumps a store's ref_count, with matching native helper in `src/codegen_runtime.rs::OpIncRc` and dispatch arm in `src/generation/dispatch.rs`; (2) `src/parser/vectors.rs::emit_lambda_code` now emits `Call(OpIncRc, [Var(v_nr)])` after each `set_field_no_check` that captures a `Reference(__cell_*, _)` into the closure record's auto-Reference attribute, so the parent's scope-exit `OpFreeRef` decrements (rc 2→1) instead of actually freeing; (3) added `pub known_type: u16` field to `Store` (default `u16::MAX`), set by interp `OpDatabase` in `src/state/io.rs` and native `OpDatabase` in `src/codegen_runtime.rs` right after `claim` succeeds, with worker stores inheriting via `clone_locked` / `clone_locked_for_worker` / `borrow_locked_for_light_worker`; (4) `Stores::free_named` now cascade-frees `Parts::DbRef` fields of stores whose `known_type` resolves to a `__closure_*` type — gated on the prefix because user code can't synthesise `__` identifiers and other `Parts::DbRef` users (P213 ChildRec) must NOT cascade.  Pinned by `c_d4_factory_independent_state` (un-ignored, `cross_mode!`) in `tests/mut_closure_matrix.rs` + `p22_phase03_multi_factory_no_leak` (NEW 100-iter × 2 factories) in `tests/leak.rs`.  Single-factory leak guard `p22_phase03_factory_no_leak` is now leak-free (was warning "100 stores not freed" between commits 2 and 4). | Medium (closed) | n/a — fix lands in `default/01_code.loft` (OpIncRc), `src/fill.rs` (regen), `src/codegen_runtime.rs` (OpIncRc helper + OpDatabase known_type), `src/generation/dispatch.rs` (OpIncRc arm), `src/parser/vectors.rs` (emit_lambda_code emission), `src/store.rs` (known_type field + worker inheritance), `src/state/io.rs` (interp OpDatabase known_type), `src/database/allocation.rs` (cascade-free in free_named). |
| 258 | Plan-22 phase 02c (Case B auto-Reference) — when a closure stored in a struct field mutates a captured struct, interp correctly propagated the mutation to the outer scope but native diverged (outer read stale value).  Reproducer: `struct State { x: integer } struct Loop { cb: fn() } fn main() { s = State{x:0}; loop = Loop{cb: fn(){s.x=13;}}; loop.cb(); println("{s.x}"); }` — interp printed 13, native printed 0.  D1 (closure in local var) and D2 (closure passed as fn arg) both worked on native; only D3 (struct field) diverged.  Surfaced 2026-05-12 by phase 02c verification (`tests/mut_closure_matrix.rs::b_d3_ref_capture_field_mutates`).  **Closed (2026-05-12)**: root cause was a layout mismatch between native and interp.  Phase 02b's `typedef.rs::fill_database` branch maps auto-Reference fields (Reference with non-empty deps) to `database.dbref()` (12-byte Parts::DbRef storage).  But the native codegen path in `src/generation/mod.rs::emit_field` had no matching branch — it fell through to the catch-all `db.field(s_var, field_name, kt_ref)` using the inner struct's `known_type` (which sized the field as inline-bytes, e.g. 8 bytes for `State{x:integer}`).  When `claim_child_rec` byte-copied the closure record's payload using native's smaller size, it truncated the auto-Reference field's 12 bytes to 8, leaving `pos=0` instead of `pos=8` and the lambda's `OpGetDbRef` reading garbage.  Fix: add a `Type::Reference(_, deps) if !deps.is_empty()` arm to `emit_field` that emits `db.field(s_var, field_name, db.dbref())` matching `typedef.rs`'s branch.  Both backends now agree on the closure record's layout; the byte-copy preserves all 12 bytes; the lambda reads the correct DbRef and mutations propagate.  Pinned by `tests/mut_closure_matrix.rs::b_d3_ref_capture_field_mutates` (interp + native both green, closing the cross_mode cell deferral note). | Medium (closed) | n/a — fix lands in `src/generation/mod.rs::emit_field` (auto-Reference branch). |
| 257 | Capturing a `vector<T>` into a closure body crashed both backends with no clean parse-time rejection.  Reproducer: `fn main() { items = [10, 20, 30]; f = fn(idx: integer) -> integer { items[idx] }; r = f(1); println("r={r}"); }` — interp panicked with `Write to locked store at rec=8 fld=2` (src/store.rs:963); native rejected with rustc E0308 + E0605 (the generated code cast an unsupported tuple-shaped value as i32).  Surfaced 2026-05-12 by plan-15 phase 06 closeout probing.  **Closed (2026-05-12)** with option (a) from the original P257 fix-shape list — added a parse-time rejection in `src/parser/objects.rs::resolve_name`'s capture-context arm.  When the capture's type is `Type::Vector` / `Hash` / `Sorted` / `Index` / `Spacial`, emit a clear diagnostic naming the variable + the recommended workaround: "<kind> variable '<name>' cannot be captured into a closure body; bind the element you need before the lambda (e.g. `x = <name>[i]; f = fn(...) { ... x ... }`) — collection capture is not supported because the closure record layout doesn't model the content type".  Both backends now produce the same parse-time error; no runtime panic, no rustc rejection.  Option (b) — implementing vector-capture support — was deferred; the closure-record layout would need extension to hold a DbRef alongside the vector's content type, and no user code in lib/* depends on capturing collections.  Pinned by `tests/parse_errors.rs::p257_vector_capture_in_closure_rejected` (asserts the diagnostic exactly) + `p257_bind_before_lambda_workaround_works` (asserts the recommended workaround compiles + runs).  Symmetric coverage for Hash / Sorted / Index / Spacial lives in the gate predicate itself rather than separate test cells (their construction syntax triggers pre-existing unrelated diagnostics that mask the closure-capture rejection at the assertion layer). | Low (closed) | n/a — fix lands in `src/parser/objects.rs::resolve_name` capture-context arm. |
| 256 | `tests/strings.rs::utf8_index` panicked with `"index 7 out of bounds for length 7"` at `tests/testing.rs:431` when the test asserted `a[0] + a[1] + a[2] + a[3] + a[4] + a[5] + a[6] + "." + a[7]` for `a = "♥😃"` (7-byte UTF-8 string) produces `"♥♥♥😃😃😃😃."`.  Surfaced 2026-05-11 during the P253 hash-DoS regression sweep; pre-existing on this branch.  Triage (2026-05-12): the test was written against the old `text[i]` OOB semantics (returned char(0) silently, which concatenated as the empty string).  Plan-07 phase 4 step 4.8 (commit 6016655e, 2026-05-11) intentionally changed `text[i]` to raise `IndexOutOfBounds` for `i >= len`, with an explicit `tests/runtime_errors.rs::kind_index_out_of_bounds_text_prints_pretty_error` cell pinning the new behaviour.  The `utf8_index` test's `+ "." + a[7]` tail assumed the silent-empty path and panicked under the new raise path.  **Closed (2026-05-12)**: removed the OOB tail (`+ "." + a[7]`) from the assertion and dropped the matching `.` from the expected string, so the test now exercises ONLY the in-range byte indices `a[0]..a[6]` (its actual purpose — verifying byte-level char indexing maps each UTF-8 byte to its containing codepoint: bytes 0-2 → ♥, bytes 3-6 → 😃).  Out-of-range coverage stays in `runtime_errors.rs::kind_index_out_of_bounds_text_prints_pretty_error`.  Test docstring now cross-references the runtime-errors cell so a future reader doesn't re-add an OOB index expecting silent return. | Low (closed) | n/a — fix lands in `tests/strings.rs::utf8_index`. |
| 255 | Capturing the result of a generic-fn `vector<T>` return into a local variable rejects with `"Variable X cannot change type from vector<S> to vector<S>"` — both sides of the diagnostic show the IDENTICAL type.  Reproducer: `struct P { v: integer } fn make<T>(x: T) -> vector<T> { o: vector<T> = []; o += [x]; o } fn main() { p = P{v:1}; vp = make(p); }` rejected on parse with the equal-types diagnostic; even a separate-line construction (`p = P{...}; vp = make(p);`) hit the same path.  Surfaced 2026-05-11 during P241 slice-3 regression test development.  Independent of P241 — the diagnostic fired before any IR rewrite or codegen ran.  Initial diagnosis ("change_var Vector→Vector arm rejects identical types") was incomplete; investigation revealed THREE coordinated root causes, each of which had to be fixed for the struct-T pattern to work end-to-end.  **Root cause 1 (the equal-types reject)**: when the call argument was a struct literal `P { v: 99 }` the expression's type carried `Type::Rewritten(Reference(P))` (the wrapper used to mark "rewritten into Insert ops"); that wrapper propagated into the bound T, then into `vector<T>`, and `Type::is_equal` did not look through `Rewritten` so a `vector<Rewritten<Reference<P>>>` LHS did not match a `vector<Reference<P>>` RHS.  **Root cause 2 (post-fix runtime garbage)**: stripping the wrapper in `is_equal` made the parse succeed but the runtime returned `4294967202` for `vp[0].v` instead of `99`.  The `Rewritten(Reference(P))` was still flowing through `resolve_type_var` into the bound T, so `vector<T>`'s element type was `Rewritten(Reference(P))` — codegen still mis-handled it.  **Root cause 3 (parametric type-id inside OpCopyRecord)**: even with T cleanly bound to `Reference(P)`, the parametric template's vector-element-write triplet `OpNewRecord(out, t_T, …); OpCopyRecord(src, elm, t_T_known); OpFinishRecord(out, elm, t_T, …)` left the type-id args pointing at the parametric T's known_type after substitution.  The runtime read the wrong record size from that placeholder type-id and returned garbage.  P241 slice 1-3 had only handled primitive T (replacing OpCopyRecord with OpSetXxx); struct T fell through with the parametric type-ids intact.  **Closed (2026-05-12)** with three coordinated fixes: (a) `src/data.rs::Type::is_equal` strips `Type::Rewritten` wrappers on either side before unifying — Rewritten is a value-construction marker, not a shape difference.  (b) `src/parser/mod.rs::resolve_type_var` strips `Rewritten` from the concrete arg type before binding T, so the marker (which describes how a particular arg was assembled, not the data shape T represents) never enters the substituted IR.  (c) `src/parser/mod.rs::rewrite_vector_write_triplets` was extended to handle struct T (Reference): a new `is_rewritable_vector_element_target` predicate gates the rewrite on either primitive T OR struct T; for struct T the OpCopyRecord shape is kept but its `tp` arg is patched to the concrete struct's `known_type` (via `data.def(content_def_nr).known_type`) AND the surrounding OpNewRecord/OpFinishRecord `parent_tp` args get patched to `concrete_vec_tp` (already computed for the primitive path).  PreAlloc prefix gets emitted for struct T too, matching the parse-time concrete-T path's perf parity.  Pinned by `tests/issues.rs::p255_capture_generic_vector_struct_return` (interp + native both green).  Side benefit: closes P241's deferred struct-T behavioural regression test — the deferred-comment block in `tests/issues.rs` was replaced with the now-passing P255 test, and the previously-deferred `e5_d1_struct_ref_loop` / `e4_d3_field_closure_local` cells in `tests/tuple_matrix.rs` un-defer cleanly. | Medium (closed) | n/a — fix lands in `src/data.rs::Type::is_equal` + `src/parser/mod.rs::resolve_type_var` + `src/parser/mod.rs::rewrite_vector_write_triplets` + `Self::is_rewritable_vector_element_target`. |
| 254 | **Security — local cache poisoning.**  `loft --native script.loft` checks `.loft/cache/<source_stem>-<source_hash>` (`src/main.rs:2286`) and, if the file existed, executed it directly via `Command::new(&binary)` (`src/main.rs:2485`) without verifying that loft itself produced it.  The filename is content-addressed (`<u64 hash>` of source bytes + `--native-release`/`--native-debug` flags + `libloft.rlib` mtime + per-package native rlib mtimes) so an attacker who knows the source AND the host's libloft.rlib mtime can pre-compute the cache filename and drop a malicious ELF there.  Pre-fix the cache directory at `<source_dir>/.loft/cache/` was created with default mode (typically `775` — group-writable on most umasks); cached binaries were mode `755` (executable).  No HMAC, no signature, no owner check at execute time.  **Threat scenarios** (low severity — local file access required): shared dev machine where a colleague drops a poisoned cache before you run; cloned project that ships a `.loft/cache/` directory unintentionally (the dir lives next to source so it travels via `git clone` unless `.gitignore`d); CI cache volumes restored from a previous job's artifact when another tenant could write to the artifact storage.  Same shape as the well-known `ccache` / `sccache` / Bazel-remote-cache poisoning class.  Surfaced 2026-05-11 during a security audit prompted by the user pointing at the hash-DoS (P253).  **Closed (2026-05-12)** with three coordinated defenses chosen from the cheapest-first list (HMAC+secret avoided — adds crypto dependency without strengthening the threat model materially; the simpler defenses cover the documented attack scenarios): (a) `cache_safe_to_execute` in `src/native_utils.rs` rejects any cached binary that is a symlink (all platforms), or that is owned by another uid, group/other-readable/writable, or carries SUID/SGID bits (Unix); the cache lookup site in `src/main.rs:2286` uses this check to decide whether to reuse the cache or recompile.  (b) `tighten_cache_dir` + `tighten_cache_binary` `chmod 0o700` the directory and freshly written binary on Unix, repairing pre-existing wider-mode caches from earlier loft versions.  (c) `LOFT_NATIVE_NO_CACHE=1` opts out of the cache entirely — bypasses lookup AND skips writing — for paranoid users who want zero on-disk artifacts.  When the safety check rejects a cached binary, loft prints `"rejecting suspicious cached binary at <path> (P254 — wrong owner, world-writable, symlink, or SUID); recompiling"` to stderr and recompiles instead of refusing service.  Cache writes that can't tighten the directory mode (e.g. NFS server-enforced mode) skip the cache write entirely with a one-line warning.  Pinned by `tests/p254_cache_poisoning.rs` (5 integration cells: fresh-cache permissions, second-run reuse, group-writable rejected, drop-poisoned-binary-not-executed, `LOFT_NATIVE_NO_CACHE=1` skips writes) + `src/native_utils.rs::p254_cache_safety` (5 unit cells covering each rejection axis).  Owner-spoof testing (file owned by a different uid) requires root to set up portably and is intentionally skipped — the `cache_safe_to_execute` helper's uid check is exercised indirectly by the integration tests' freshly-written-by-self-uid path. | Low (closed) | n/a — fix lands in `src/native_utils.rs` (helpers + unit tests), `src/main.rs:2286-2330` (cache-lookup safety + `LOFT_NATIVE_NO_CACHE=1` opt-out), `src/main.rs:2466-2510` (cache-write safety + dir/file mode tightening), `tests/p254_cache_poisoning.rs` (integration tests). |
| 253 | **Security — hash-table collision DoS.**  `src/keys.rs:325` used `std::collections::hash_map::DefaultHasher::new()` which constructs a SipHash-1-3 hasher with **fixed seed (k0=0, k1=0)**.  Every loft process used the identical hash function, so an attacker who could supply hash-table keys (HTTP query params, JSON object keys, log-tag strings) could pre-compute N strings that all collide to a single bucket → O(N²) insertion / lookup.  Rehash on growth (`src/hash.rs:21-41`) did NOT break the cluster: it used the same fixed-seed hash function.  Same root-cause class as the 2011/2012 hash-DoS in Python / Ruby / PHP / Java / Node.js (CVE-2011-4815 et al.).  Severity bounded by who can supply keys: today's `lib/web` accepts attacker-controlled strings (HTTP query params, JSON body, WebSocket frames).  Surfaced 2026-05-11 by the user during a security audit.  **Closed (2026-05-11)**: replaced `DefaultHasher::new()` in `src/keys.rs::hash` and `key_hash` with `build_hasher()`, which calls a process-wide seeded `RandomState` memoised via `OnceLock`.  `RandomState::new()` seeds from `getrandom` on first call (per stdlib's documented behaviour); the OnceLock memoisation ensures subsequent hashers share the same seed (otherwise resize / lookup would see a different distribution than insertion — the bucket cluster the attacker built would no longer be reachable, but neither would legitimate keys).  Same shape as `HashMap`'s default hasher.  Hash collection iteration order changes per-process (was already documented unordered, so no user-visible breakage); no regression in 631/631 issues, 47/47 wrap, 42/42 tuple_matrix tests.  Pinned by `tests/issues.rs::p253_hash_remains_functional_after_seeding` (smoke check that a hash collection still inserts + looks up correctly under the seeded hasher); the cross-process variance property is a stdlib `RandomState` guarantee that's not directly testable in-process. | Low (closed) | n/a — fix lands in `src/keys.rs::hasher_state` + `build_hasher`. |
| 252 | Bounded-generic for-loop over a struct-ref vector: the bound method dispatched on the loop variable returns the FIRST item's result for every iteration, instead of the per-item result.  Reproducer: `interface V { fn ok(self: Self) -> boolean } struct P { v: integer } fn ok(self: P) -> boolean { return self.v > 0; } fn check<T: V>(items: vector<T>) { for it in items { print("v?={it.ok()}\n"); } } fn main() { check([P{v:1}, P{v:0}, P{v:3}]); }` printed `v?=true / v?=true / v?=true` (3× true) instead of `v?=true / v?=false / v?=true`.  Bisected to slice-3 commit `6016655e` (vec/text OOB raise + Nullable opcode peers); the slice-3 reroute swapped `OpGetVector` → `OpGetVectorNullable` in the for-loop iter step (parser/collections.rs:217-237).  **Closed (2026-05-11):** the I9-vec elm_size fixup in `src/parser/mod.rs::substitute_type_in_value` only matched the literal name `"OpGetVector"` — after the slice-3 swap the bounded-generic for-loop's iter step was left at `OpGetVectorNullable(v, 0, idx)` with size=0, so every iteration read element 0 → bound method always saw the FIRST item.  Fix: extend the name match to also accept `"OpGetVectorNullable"`; both peers have identical `(r, size, idx)` arg shapes so the existing fixup logic applies unchanged.  Pinned by `tests/issues.rs::p252_bounded_generic_for_loop_per_item_dispatch` (interp + native both green).  Also re-enables `tests/scripts/86-interfaces.loft::test_bounded_for_loop_struct` under `--native` — the only remaining native-suite failure is now closed. | Medium (closed) | n/a — fix lands in `src/parser/mod.rs::substitute_type_in_value` |
| 251 | Storing a tuple whose element is a `Type::Function` (closure / fn-ref) INTO a struct field failed native compilation with rustc E0605 `(u32, DbRef) as i32`.  Reproducer: `struct S { t: (fn(integer) -> integer, integer) } fn main() { add5 = fn(x: integer) -> integer { x + 5 }; s = S { t: (add5, 99) }; print("{s.t.1}\n"); }` — interp printed `99`, native rejected with `let _v_val = (var_add5); ... _v_val as i32` where `var_add5` is the `(u32, DbRef)` runtime fn-ref tuple — the projection `.0` to get the u32 d_nr was missing on this code path even though P196 added it for the OTHER fn-ref-in-tuple-in-struct shape.  The call-through-field shape (`s.t.0(arg)`, also `f = s.t.0; f(arg)`) compounded the issue under interp with "index out of bounds: the len is 3 but the index is 10" at `src/database/allocation.rs:162` (calling a wildly-out-of-range d_nr because the storage bug fed garbage into the call dispatch).  Surfaced 2026-05-11 during plan-14 phase 05 D3 cell write-out.  **Closed (2026-05-11):** root cause was that `src/parser/mod.rs::emit_set_one_element`'s `Type::Function` arm only special-cased `Value::FnRef` literal — when the source was `Value::Var(v)` with `v` typed `Type::Function`, the value passed through unchanged.  Native emit then produced the broken `(var_v) as i32` shape.  Fix: extend the arm to also wrap `Value::Var(v)` in `Value::FnRefDnr(v)` when `v` has type `Function` AND is not in the closure-vars table (non-capturing).  `Value::FnRefDnr` natively emits `(var_v.0 as i64)` projecting the d_nr half; bytecode emits `OpVarInt` reading 8B from the slot (first 8B is the i64 d_nr).  Mirrors the existing projection in `emit_fn_ref_field_write` (parser/mod.rs:4886) for the direct-field-write path — extending it to the tuple-element-of-struct-field path closes the remaining gap.  Both store-only (`s.t.1` read-back) and call-through-field (`s.t.0(arg)`, `f = s.t.0; f(arg)`) shapes work on both backends.  Pinned by `tests/issues.rs::p251_tuple_with_fnref_in_struct_field_read` + `_call`, plus the now-un-deferred `tests/tuple_matrix.rs::e4_d3_field_closure_local` cell. | Medium (closed) | n/a — fix lands in `src/parser/mod.rs::emit_set_one_element` `Type::Function` arm. |
| 250 | Tuple-of-`Reference` returned from a function and destructured inside a loop body showed a **stale-DbRef on the destination variable that picked up the FIRST argument** on iterations >0.  Reproducer: `struct P { v: integer }; fn make_pair(a: P, b: P) -> (P, P) { (a, b) } fn main() { for i in 0..5 { pa = P{v:i}; pb = P{v:i+100}; (q1, q2) = make_pair(pa, pb); print("{i}: {q1.v},{q2.v}\n"); } }` printed `0: 0,100` then `1: null,101` `2: null,102` … on BOTH backends.  `q1` (= `a`'s ref, the first arg) read `null` once the loop body re-entered its scope; `q2` stayed correct because by the time q2 was read the new tuple was being built from valid `pa`/`pb` allocations — the read happened before the second iter's overwrite.  Surfaced 2026-05-11 during plan-14 phase 04 pre-flight.  **Closed (2026-05-11):** root cause was that the destructure code in `src/parser/expressions.rs` (synthetic-`__tuple<…>` path, lines 1252-1278) emitted the LHS Reference vars (q1, q2) as `OpGetField(tmp, offset, ...)` reads — DbRefs that share `store_nr` with the outer `tmp` variable.  Without dep tracking, scope analysis emitted an independent `OpFreeRef` for q1 and q2 at scope exit; each free works on a `store_nr` basis and reclaimed the entire tuple's underlying store on the FIRST exit.  The next loop iteration's `tmp = make_pair(...)` reassignment then ran `OpFreeRef(tmp)` on the now-stale outer DbRef whose store_nr got recycled by the next iter's `pa` allocation, silently destroying that allocation.  Position-dependent because the freshly-allocated `pa` lands in the same store slot the prior tuple occupied (FIFO recycling).  Fix: in the synthetic-struct destructure path, mark each Reference-typed LHS as `vars.depend(v_nr, tmp)` so scope analysis treats them as borrows (deps non-empty → skip `OpFreeRef`).  `tmp`'s `OpFreeRef` alone reclaims the storage at the right time.  Only applies to Reference elements; primitive elements (TupleGet path) read value-typed slots that need no free.  Pinned by `tests/issues.rs::p250_loop_destructure_first_arg` and the new tuple_matrix `e5_d1_struct_ref_loop` cell (both backends green).  All 41 tuple_matrix cells now pass. | Medium (closed) | n/a — fix lands in `src/parser/expressions.rs` synthetic-struct destructure path. |
| 245 | `parallel { server_arm(...); client_arm(...); }` hangs indefinitely when one arm performs a blocking accept on a TCP listener and the other arm tries `web::ws_handler(...)` against the same loopback port.  Two compounding root causes, both fixed together (2026-05-10): **(a)** the worker thread's stack was empty when the arm body ran, so reads at the parent's frame offsets returned garbage — `port = 18092; parallel { server_arm(port); ... }` saw `port` arrive as 0 (or whatever stale bytes were at offset 8 in the worker's freshly-`database(1000)` record).  **(b)** `src/extensions.rs::native_auto_dispatch` held the `NATIVE_SIGS` mutex guard ACROSS the native-fn invocation — when a sibling worker called a blocking native fn (`n_tcp_accept`'s `listener.accept()`), the guard stayed held; every other worker calling any auto-marshalled native fn (`web::sleep_ms`, `web::ws_handler`, etc.) blocked on `NATIVE_SIGS.lock()` indefinitely.  This serialised every parallel arm whenever one arm did blocking I/O, manifesting as the original "client thread silently never starts" symptom.  **Closed (2026-05-10)** with two coordinated changes: (1) `src/state/mod.rs::parallel_join` captures the parent's stack contents (offsets 4..stack_pos) into an `Arc<Vec<u8>>` snapshot, threaded through `run_parallel_block` to each worker; the new `State::execute_at_void_with_snapshot` overlays the snapshot on the worker's stack at offset 4 and sets `stack_pos` to mirror the parent's, so variable reads at parent-frame offsets resolve correctly.  Arms still get isolated `Stores` clones via `clone_for_worker`, so writes inside an arm don't propagate back.  (2) `src/extensions.rs::native_auto_dispatch` now clones the `(sym, sig)` entry out of the `NATIVE_SIGS` table and DROPS the mutex guard before `dispatch_call` — the lock is held only for the table lookup, never for the call.  Pinned by `tests/scripts/81-parallel-outer-vars.loft` (regression guard for fix (a)) and the existing `tests/scripts/80-parallel-block.loft` (no-regression on the simple-arm case).  The original v5 t1-t5 keep their subprocess shape — the subprocess pattern is still right for isolation and CI parallelism — but `parallel{}` with I/O now composes cleanly for plan-36's audience-server demo and any in-process loft program. | Medium (closed) | n/a — fix lands in `src/state/mod.rs` (parallel_join + execute_at_void_with_snapshot), `src/parallel.rs` (run_parallel_block snapshot threading), `src/extensions.rs::native_auto_dispatch` (mutex-guard release before dispatch). |

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
[plan 10's phase 00 characterisation](plans/deferred/10-scope-exit-emission/00-characterize.md)
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

- The `server` library (`doc/claude/lib_plans/future/08-server/README.md`) — handler
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
