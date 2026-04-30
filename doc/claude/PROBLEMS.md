
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
| 195 | Chained literal field indexing `n.v.0.0` mis-parses — the lexer reads `0.0` as a single float literal.  Affects any nested-tuple access where two consecutive integer indices appear without an intervening identifier. | Low | Stash the inner element first: `inner = n.v.0; inner.0`. |
| 196 | Native codegen for `(fn(int) -> int, int)` (or any tuple containing a fn-ref) fails with `(u32, DbRef).0 as i32` — the fn-ref tuple element's runtime shape doesn't fit the OpSet/OpGet narrowing path used for primitive ints.  Interpreter mode works; only native compilation breaks.  Closes via [ARC.md A6.c](plans/06-typed-par/ARC.md) (4d.C closure-storage redesign). | Medium | Use a struct field for fn-ref instead of tucking it in a tuple: `struct H { f: fn(...) -> ..., n: int }`. |
| 198 | `tests/scripts/95-alias-copy.loft` leaks Database 3 (allocated by `OpInitRef` at pc≈4788) — `p146_script_95_alias_copy_leak` regression test panics on `roadmap-lsp-eclipse`.  Passes on main; the regression sits in commits between `main` (`05b53b2`) and `roadmap-lsp-eclipse` HEAD.  Most likely culprits: plan-04/05 slot allocator refit, plan-06 par-safety series, or the plan-07 Span IR walker arms missing the alias-copy free path.  Investigated as part of [ARC.md A1](plans/06-typed-par/ARC.md). | High | None at the loft language level — the test catches a runtime invariant. Investigate scopes.rs `scan_set` aliased-return free-emission against the new IR variants (`Value::Span`, `Value::ParFor`) added on this branch. |
| 199 | Native codegen for `n_assert(stores, n_add_pair(stores, var_p) == 30, …)` emits two simultaneous `&mut stores` borrows (E0499).  Reproducer: `tests/scripts/50-tuples.loft` line 21; `cargo test --release --test native native_tuple_script`.  Passes on main, fails on `roadmap-lsp-eclipse` — regression on this branch.  Blocks native-mode tuple par; see [ARC.md A7 risk register](plans/06-typed-par/ARC.md). | Medium | Hoist the inner call into a temporary: `let r = add_pair(p); assert(r == 30);` makes the second borrow fall outside `n_assert`'s argument list. |
| 200 | Native codegen for `f += <integer>` against a binary file (`BigEndian` / `LittleEndian` open mode) emits a value with mismatched expected width — `rustc` raises E0308 mismatched types.  Reproducer: `tests/scripts/20-binary.loft` line 67 / 128.  Passes on main, fails on `roadmap-lsp-eclipse` — regression on this branch. | Medium | Add the explicit width cast the parser warning already suggests: `f += val as i32` (or `as i8` / `as u32` etc.). |
| 201 | `tests/html_wasm.rs` Mutex-poison cascade: when one test panics inside `build_lock().lock().unwrap()` (line 174), every subsequent html_wasm test fails with the unhelpful `called Result::unwrap() on an Err value: PoisonError { .. }` instead of the original error message.  The `--html` driver writes to a fixed `/tmp/loft_html.rs` so the lock is genuinely needed; the issue is that a poisoned lock should report the original panic, not be re-unwrapped naively. | Low (test infra) | Use `lock().unwrap_or_else(\|e\| e.into_inner())` to recover from poison; the cascade then surfaces the actual first failure instead of hiding it.  Or have `assert_wasm_rlib_fresh()` fail the test before acquiring the lock so a stale rlib doesn't poison the build serial. |

## Interpreter Robustness

### 195. Chained literal indexing — lexer reads `0.0` as float

**Symptom:** `n.v.0.0` (read element 0 of inner tuple of element 0)
fails to parse:

```loft
struct Nested { v: ((integer, integer), (integer, integer)) }
fn test() {
  n = Nested { v: ((1, 2), (3, 4)) };
  a = n.v.0.0;                    // ← parse error: float `0.0`
                                  //   followed by stray `.` ?
}
```

**Where:** `src/lexer.rs` greedy-reads `<digit>.<digit>` as a single
floating-point number token.  The post-field path doesn't unread the
fractional part when the previous token was a `.` separator.

**Fix path:** in the lexer's number-reading routine, when the previous
non-trivia token was `.` (or, more conservatively, when a tuple-index
context is pending), treat `<digit>` followed by `.` as a single-digit
integer rather than the start of a float.  The cleanest mechanism is
an "integer-only" lexing mode the parser opts into when it knows it's
expecting a tuple index.

**Workaround:** stash the inner element first:

```loft
inner = n.v.0;
a = inner.0;     // OK
```

**Test:** add a test once fixed.

### 196. Tuple-of-fn-ref native codegen — `(u32, DbRef).0 as i32`

**Symptom:** native compilation fails with `non-primitive cast` when
a tuple struct field contains a fn-ref element:

```loft
struct C { pair: (fn(integer) -> integer, integer) }
fn dbl(x: integer) -> integer { x + x }
fn test() {
  c = C { pair: (dbl, 21) };       // interpreter: OK
                                   // native: rustc rejects
                                   //   (var_tmp.0) as i32
}
```

**Where:** my `set_field_check::Type::Tuple` arm dispatches the
fn-ref element to `OpSetInt4(ref, pos, TupleGet(tmp, i))`.  At native
emit, `TupleGet(tmp, i)` for a fn-ref element resolves to
`var_tmp.0` whose Rust type is `(u32, DbRef)` (the native fn-ref
representation).  `OpSetInt4`'s `#rust` body wraps the value with
`as i32`, which rustc rejects on a tuple type.

**Fix path:** in `set_field_check::Type::Tuple`, when the element is
`Type::Function`, extract the fn-ref's `d_nr` (`var_tmp.i.0 as u32`,
then `as i32` is fine on `u32`) before passing to `OpSetInt4`.  The
top-level `Type::Function` set arm already handles `Value::FnRef` ↔
`Value::Int(d_nr)` reduction; extend that to also unwrap a Var-of-
fn-ref when the LHS is a fn-ref tuple element.  Alternatively, emit
a small helper like `OpGetFnRefDnr(var, idx)` returning `i32`.

**Workaround:** lift the fn-ref out of the tuple into its own
struct field:

```loft
struct C { f: fn(integer) -> integer, n: integer }
```

**Test:** `tests/issues.rs::p4d_fn_ref_as_struct_field` covers the
top-level case.  Add `p4d_tuple_field_with_fn_ref` once fixed.

**Tracked in plan-06:** closes in [ARC.md A6.c](plans/06-typed-par/ARC.md)
once the 4d.C closure-storage redesign lands (the 16-byte (d_nr,
closure DbRef) layout makes the storage rep match the native
`(u32, DbRef)` byte-for-byte, eliminating the cast mismatch).

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

**Where:** `tests/scripts/50-tuples.loft` line 21:
`assert(add_pair(p) == 30, …)`.  Native codegen lowers
`assert(expr, msg)` to `n_assert(stores, expr_eval, msg_eval)` and
inlines both arguments — when the inner expression itself takes
`&mut stores`, rustc rejects.

**Branch state (2026-04-29):** passes on `main`, fails on
`roadmap-lsp-eclipse`.  Regression on this branch.

**Fix path:** in the native code generator, hoist any inner call
that takes `&mut stores` into a temporary `let` before the outer
call.  The fix mirrors what the loft-level value-semantics layer
already does for the interpreter — native codegen needs an explicit
sequence-point splitter for nested `&mut stores` consumers.  Files:
`src/generation/emit.rs` (or wherever native call args are emitted).

**Workaround:** rewrite the loft source as
`r = add_pair(p); assert(r == 30, …);` so the second borrow leaves
`n_assert`'s argument list.

**Test:** `tests/native.rs::native_tuple_script` (and
`native_tuple_return_script` — same fingerprint).

**Tracked in plan-06:** [ARC.md A7](plans/06-typed-par/ARC.md) needs
this fixed before tuple par compiles natively.  A7 covers
interpreter mode first; native-mode tuple par becomes a follow-up
(A7 risk register).

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
