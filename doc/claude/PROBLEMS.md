
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
| 202 | Native codegen for `for ... par(...)` for-loops calls `n_parallel_queue` (and `_text` / `_ref` / `_narrow` / `_fn` variants) which only exist in the bytecode interpreter — no `codegen_runtime.rs` implementation.  Reproducer: `tests/docs/19-threading.loft` → `cargo test --release --test native native_dir`.  Generated stub takes 5 args (loft signature); call site passes 6+ (with `n_extra` count); arg-count mismatch breaks rustc compile.  Distinct from P199 — purely a missing native implementation, not a borrow-conflict.  Also unblocks 19_threading + any par-queue use in native scripts.  See [P202 design](#202-native-n_parallel_queue-family-not-implemented). | Medium | Compile the loft source via the interpreter (`cargo run --bin loft --release -- --interpret file.loft`) — par-for works there.  Or use the par-for-native dispatch path by writing the worker as `n_parallel_for_native` directly (advanced — bypasses the for-par syntactic sugar). |
| 200 | Native codegen for `f += <integer>` against a binary file (`BigEndian` / `LittleEndian` open mode) emits a value with mismatched expected width — `rustc` raises E0308 mismatched types.  Reproducer: `tests/scripts/20-binary.loft` line 67 / 128.  Passes on main, fails on `roadmap-lsp-eclipse` — regression on this branch. | Medium | Add the explicit width cast the parser warning already suggests: `f += val as i32` (or `as i8` / `as u32` etc.). |
| 201 | `tests/html_wasm.rs` Mutex-poison cascade: when one test panics inside `build_lock().lock().unwrap()` (line 174), every subsequent html_wasm test fails with the unhelpful `called Result::unwrap() on an Err value: PoisonError { .. }` instead of the original error message.  The `--html` driver writes to a fixed `/tmp/loft_html.rs` so the lock is genuinely needed; the issue is that a poisoned lock should report the original panic, not be re-unwrapped naively. | Low (test infra) | Use `lock().unwrap_or_else(\|e\| e.into_inner())` to recover from poison; the cascade then surfaces the actual first failure instead of hiding it.  Or have `assert_wasm_rlib_fresh()` fail the test before acquiring the lock so a stale rlib doesn't poison the build serial. |
| 203 | Assertion `delete(path) == FileResult.Ok` panicked with `delete existing file`.  **Closed (2026-05-02)**: actual root cause was template double-substitution — five templates in `default/01_code.loft` (lines 690, 705, 707, 751, 753) substituted `@v1`/`@v2` in multiple positions, so any side-effecting call appearing on both sides of an enum/null comparison evaluated twice.  Fix: `src/generation/calls.rs::output_call_template` now scans every template for repeated `@<name>` placeholders and hoists each into a single `let _v_<name> = …;` wrapping `{ … }` block before substitution, so each arg expression evaluates exactly once regardless of how many positions reference it.  `repro_p203.loft` exits 0; full suite: 540/540 issues, 43/43 threading, 35/35 threading_chars, native 86/92 → 87/92.  See [P203 reproducer + diagnosis](#203-native-block-scope-file-not-flushed-on-exit). | Medium (closed) | n/a — fix lands in `src/generation/calls.rs`. |
| 204 | Native: tail-expression `return inner_call()` from a struct-returning function emits `n_inner(cell, args)` as a void STATEMENT and then `return DbRef { store_nr: u16::MAX, rec: 0, pos: 8 };` (null sentinel).  The caller does `let _src = n_wrap(...); OpCopyRecord(_src, var_q, ...)` — `_src` is null, OpCopyRecord panics with "index out of bounds: the len is X but the index is 65535" at `src/database/allocation.rs:347`.  Native FAILS, interpreter PASSES (interpreter routes the result through the `__ref_*` placeholder mechanism which native skips).  Surfaces in `87_store_leaks` and `85_yield_resume` after P199 made the surrounding code compile.  See [P204 reproducer + diagnosis](#204-tail-expression-return-of-inner-helper-call-discarded). | Medium | Avoid the tail-expression pattern: bind the result first.  `fn wrap(x) -> S { y = make_y(); r = inner(x, y); r }` — naming the result `r` produces a different IR shape that native handles correctly. |
| 205 | Native: bounded-generic dispatch `fn f<T: Trait>(x: T) -> text { x.to_label() }` returns a `Str` whose pointer references a local `String` that's dropped on function return → dangling pointer, comparison fails.  Reproducer: `tests/scripts/86-interfaces.loft:47` — `assert(if_label(if_it) == "widget", …)` fails despite `to_label` returning `"widget"`.  See [P205 reproducer + diagnosis](#205-generic-text-return-dangles-via-str-newlocal_string).  Interpreter passes via different text-return ABI. | Medium | Avoid bounded-generic functions returning `text`.  Inline the body or use a non-generic specialisation. |

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
`doc/claude/plans/09-native-runtime-rewrite/05-file.md`
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

### 202. Native: `n_parallel_queue` family not implemented

**Symptom:** native compilation fails on any script using
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

### 204. Tail-expression return of inner helper call discarded

**Symptom:** `cargo test --release --test native native_scripts`
(slots `87_store_leaks` and `85_yield_resume`) panics:

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

### 205. Generic text return dangles via `Str::new(&local_String)`

**Symptom:** `cargo test --release --test native native_scripts`
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
`doc/claude/plans/09-native-runtime-rewrite/07-generics.md` step
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
