# Changelog

All notable changes to the loft language and interpreter.

---

## [Unreleased]

### Safety fixes

- **`const` parameter writes now panic in release builds** (S22) — The
  `#[cfg(debug_assertions)]` guard on auto-lock insertion has been removed from
  `src/parser/expressions.rs`.  `store.claim()` and `store.delete()` now use
  `assert!` instead of `debug_assert!`, so writes to `const` Reference or Vector
  parameters produce a panic in both debug and release builds.  Previously, release
  builds silently discarded the write into a dummy buffer, causing `par()` workers
  to continue with stale data.  Tests `claim_on_locked_store_panics` and
  `delete_on_locked_store_panics` in `tests/expressions.rs` verify the runtime
  enforcement.

- **`e#remove` on a generator iterator: defense-in-depth runtime guard** (S24) —
  Calling `e#remove` inside a generator `for` loop was already rejected at compile
  time (CO1.5c).  A matching runtime guard has been added to `state/io.rs::remove()`
  and `codegen_runtime.rs::OpRemove()`: if `store_nr == u16::MAX` (the coroutine
  sentinel), a `debug_assert!` fires and the call returns early, preventing
  release-build store corruption even if the compiler check is somehow bypassed.

- **Generator functions rejected as `par()` workers at compile time** (S23) — The
  parser now detects when a `par()` worker function has return type `iterator<T>` and
  emits a clear diagnostic instead of allowing the call to proceed.  At runtime,
  worker threads have their own (empty) coroutine table; passing a generator DbRef
  across thread boundaries would either panic with an out-of-bounds index or silently
  advance the wrong generator.  A runtime bounds guard in `coroutine_next` provides
  defence-in-depth.  Test `par_worker_returns_generator` in `tests/parse_errors.rs`
  covers the compile-time path.

- **Exhausted coroutine slots freed immediately** (S26) — `coroutine_return` now sets
  the slot to `None` after marking it `Exhausted`, so the `State::coroutines` Vec does
  not grow without bound across repeated `for n in gen() { }` loops.  A guard in
  `coroutine_next` handles the `None` case (push null, return) so existing code that
  re-iterates is unaffected.  Test `coroutine_frame_freed_after_exhaustion` in
  `tests/expressions.rs` runs 1 000 loops to confirm no slot leak.

- **Coroutine `text_positions` save/restore across yield (debug builds)** (S27) —
  In debug builds, `coroutine_yield` now saves the suspended frame's
  `text_positions` entries and removes them from the live set; `coroutine_next`
  restores them on resume.  This prevents false double-free warnings and
  mask-missing-free bugs in `TextStore` ownership tracking when a generator is
  interleaved with text operations in the caller.  Test
  `coroutine_text_positions_save_restore` in `tests/expressions.rs`.

- **`WorkerStores` newtype for compile-time worker-store isolation** (S30) —
  `clone_for_worker` now returns `WorkerStores` instead of plain `Stores`.
  `WorkerStores` is `Send` but not `Sync` (via `PhantomData<*mut ()>`), giving a
  compile-time guarantee that worker-thread store snapshots are passed exclusively to
  `State::new_worker` and cannot be aliased across threads.  A `Deref<Target = Stores>`
  impl allows existing test code to inspect fields without change.

- **Debug generation counter for stale-DbRef detection in coroutines** (S28) —
  `Store` now carries a `generation: u32` field (debug builds only), incremented on
  every `claim`, `delete`, and `resize` call.  `coroutine_yield` snapshots the
  generation of every live, unlocked store; `coroutine_next` asserts that no snapshot
  store changed between yield and resume.  This catches the stale-DbRef hazard — where
  a struct record held by a suspended generator is freed or reallocated by the caller —
  as an early `debug_assert!` panic rather than silent corruption.  Test
  `coroutine_stale_store_guard` in `tests/expressions.rs`.

- **Parallel worker stores use `thread::scope` and skip `claims` clone** (S29) —
  `run_parallel_direct` in `src/parallel.rs` now uses `thread::scope` instead of
  `thread::spawn` + manual join loop, giving lifetime-bounded joining with no `Vec`
  of handles.  `Store::clone_locked_for_worker` skips cloning the `claims` `HashSet`
  (workers never call `validate()`) and `store.valid()` skips the claims check for
  locked stores, removing a spurious "Unknown record" panic that appeared in debug
  builds when workers accessed struct fields.

- **Store allocator uses free-bitmap; non-LIFO slot reuse now correct** (S29 P1-R4) —
  `database_named` previously always allocated from `self.max` and only reclaimed the
  top slot on `free_named`.  Native `OpFreeRef` legitimately frees slots in non-LIFO
  order, leaving freed slots permanently wasted and `max` growing without bound.  A
  `free_bits: Vec<u64>` bitmap was added to `Stores`; `set_free_bit`/`clear_free_bit`
  helpers update it on every free/alloc, and `find_free_slot` scans for the lowest set
  bit below `max`.  `clone_for_worker` propagates the bitmap to worker stores.
  Test `store_non_lifo_free_reclaims_slot` in `tests/threading.rs` verifies that a
  freed non-top slot is reused by the next `database()` call and `max` does not grow.

### Native test harness fixes

- **`any`, `all`, `count_if` now work in native code generation; `47-predicates.loft` and `46-caveats.loft` unskipped** (N8a.4) —
  `predicate_loop_scaffold` in `src/parser/collections.rs` previously wrapped
  `[for_next, break_if_done]` in a `v_block`, which in native codegen became a
  Rust `{ ... }` block.  The loop variable (`any_elm`, `all_elm`, `cntif_elm`) was
  declared inside that block, making it invisible to the `short_circuit` or
  `count_step` expression that followed outside the block.  The fix inlines
  `for_next` and `break_if_done` directly in the loop body (the scaffold now returns
  a 4-tuple instead of 3), eliminating the nested block.  Both `47-predicates.loft`
  and `46-caveats.loft` (which uses `any`/`all` internally) removed from
  `SCRIPTS_NATIVE_SKIP`.

- **`45-field-iter.loft` stale skip removed from native test harness** (N8a.5) —
  The `// A10` skip entry for `45-field-iter.loft` in `SCRIPTS_NATIVE_SKIP` was
  stale: the field-iteration native backend already worked correctly after the A10
  implementation.  The entry has been removed; `45-field-iter.loft` now runs in the
  `native_scripts` test alongside all other unblocked scripts.

- **Tuple types now supported in native code generation; `50-tuples.loft` unskipped** (N8a) —
  Three complementary fixes enable tuple types in the `--native` backend:
  (N8a.1) `rust_type(Type::Tuple)` now emits the correct Rust type `(T0, T1, …)`
  instead of `()`, and `default_native_value` returns `String` so tuple zero-values
  `(0, 0)` are built dynamically.
  (N8a.2) `Value::TupleGet` in `emit.rs` now uses the variable's declared name instead
  of its internal index number; `Value::TuplePut` emits the actual element assignment
  `var_x.i = …` rather than a stub.  `TuplePut` added to `is_void_value` in
  `pre_eval.rs` so the block emitter treats it as a statement, not a return expression.
  (N8a.3) Tuple-returning functions `make_pair`/`swap_pair` added to
  `tests/scripts/50-tuples.loft` (with LHS destructuring); the script removed from
  `SCRIPTS_NATIVE_SKIP`.  Both interpreter and native backends pass all tuple assertions.

- **Slot conflict in `20-binary.loft` fixed; removed from native skip list** (S32) —
  `adjust_first_assignment_slot` in `src/state/codegen.rs` now checks for same-scope
  sibling overlap (`has_sibling_overlap`) before moving a variable down to TOS, mirroring
  the existing `has_child_overlap` guard for child-scope variables.  This prevented `rv`
  and `_read_34` in `n_main` from being assigned the same slot range `[820, 832)` despite
  overlapping live intervals.  `20-binary.loft` removed from `SCRIPTS_NATIVE_SKIP`.

- **Generic instantiation confirmed working in native backend; `48-generics.loft` unskipped** (N8c) —
  Audit (N8c.1) showed that monomorphised generic functions already emit correct native
  code.  `48-generics.loft` removed from `SCRIPTS_NATIVE_SKIP`.

- **Optional feature dependencies now passed to standalone `rustc`** (S31) — The
  native test harness now calls `collect_extra_externs()`, which scans all `.rlib`
  files in the current test binary's `deps/` directory and passes each as
  `--extern crate_name=path`.  This unblocks scripts that use `rand`, `rand_seed`,
  or `rand_indices`: `tests/scripts/15-random.loft` and `tests/docs/21-random.loft`
  have been removed from the native skip lists.

- **Native rlib lookup now uses the current test binary's profile** (S33) — The
  previous `find_loft_rlib()` compared modification times across `release/` and
  `debug/` deps directories and could select the wrong profile's rlib (e.g. a
  newer no-features rlib from a `--no-default-features` CI step).  The function
  now uses `current_exe().parent()` — always the current test binary's own `deps/`
  directory — so the selected rlib always matches the features the test was compiled
  with.  `tests/docs/14-image.loft` has been removed from `NATIVE_SKIP`.

### New features

- **Mutable closure capture works** (A5.6a) — `count += x` inside a lambda now
  compiles and executes correctly.  The `+=` operator on a captured integer variable
  routes through `call_to_set_op` → `OpSetInt`, bypassing the `generate_set`
  self-reference guard that previously caused a codegen panic.  Test `capture_detected`
  in `tests/parse_errors.rs` passes without `#[ignore]`.  Text capture remains
  blocked by two runtime bugs (see CAVEATS.md C1).

- **Lambda function type no longer includes text work variables** (A5.6a fix) —
  `parse_lambda` previously built the `Function(params, ret)` type from
  `data.attributes(d_nr)`, which also includes internal text work variables
  registered by `text_return()`.  This caused spurious "expects N argument(s),
  got M" errors when calling text-returning lambdas via function references.  The
  type is now built directly from the declared `arguments` list, which is always
  correct regardless of how many work variables are registered.

- **Closure capture works in debug builds** (A5.6) — The debug-mode store leak
  where closure record variables (`___clos_N`) were never freed has been fixed.
  `scopes.rs` now pre-registers block-result Reference variables at the enclosing
  outer scope so `get_free_vars` emits `OpFreeRef` at function exit.  A compile-time
  checker (`check_arg_ref_allocs`) panics in debug builds if any `Set(ref, Null)`
  initialisation is still nested inside a call argument, catching this class of
  scope-registration bug early.  Tests `closure_capture_integer`,
  `closure_capture_multiple`, and `closure_capture_after_change` all pass without
  `#[ignore]` in both debug and release builds.  Text capture and mutable capture
  remain deferred (A5.6 in ROADMAP.md).

- **`yield from` slot-assignment regression fixed** (CO1.4-fix) — `yield from
  inner()` inside a coroutine with local variables before the delegation now
  produces correct results.  The two-zone slot redesign (S17/S18) already
  eliminated the overlap between the `__yf_sub` handle and inner loop
  temporaries; no additional IR restructuring was required.  Test
  `coroutine_yield_from` passes without `#[ignore]`.

- **`stack_trace()` works in parallel workers** (S21, fix #92) — Calling
  `stack_trace()` inside a `par(...)` loop body or any `run_parallel_*` worker
  now returns the actual call frames instead of an empty vector.  Two changes
  enable this: (1) `WorkerProgram` now carries `stack_trace_lib_nr` so the
  resolved index of `n_stack_trace` travels from the main state into each
  worker state; (2) `static_call` takes the call-stack snapshot when
  `stack_trace_lib_nr` matches even when `data_ptr` is null, using a
  `"<worker>"` placeholder for frames that lack `Data` context.  Worker states
  created via both `n_parallel_for_int` (bytecode path) and the direct
  `run_parallel_*` Rust API now report correct frame counts.  Test
  `parallel_stack_trace_non_empty` passes without `#[ignore]`.

- **`init(expr)` circular dependency detection** (S20) — Struct fields that
  form a mutual initialisation cycle (`a: integer init($.b), b: integer init($.a)`)
  now produce a compile error naming the cycle (e.g.
  `circular init dependency: a -> b -> a`).  A DFS cycle check runs after all
  struct fields are parsed; `$.field` reads inside `init(...)` are tracked by
  the parser and checked for cycles per root field.

- **`stack_trace()` vector fields zeroed + call-site line numbers** (S19) —
  `stack_trace()` now returns correct call-site line numbers (`StackFrame.line`)
  for every frame.  Three fixes: `n_stack_trace` explicitly zeroes the
  `arguments` and `variables` fields of each `StackFrame` element so that
  reused store blocks don't leave garbage data; `execute_log_steps` now
  pushes the same synthetic entry `CallFrame` as `execute_argv` (Fix #88
  parity); `fn_call` now resolves call-site lines with a BTreeMap backward
  range search, recovering the correct source line even when `code_pos` has
  advanced past the `line_numbers` entry.
  Tests `stack_trace_returns_frames`, `stack_trace_function_names`, and
  `call_frame_has_line` all pass without `#[ignore]`.

- **Tuple text elements** (T1.8b) — Functions returning `(integer, text)` (or any
  tuple containing a `text` element) now compile and execute correctly.  Text elements
  are stored as `Str` (16B borrowed reference) in tuple slots via the new `OpPutText`
  opcode, consistent with loft's text-argument convention.  Four codegen sites were
  updated: null-init now emits `OpConvTextFromNull`; slot stores use `OpPutText` instead
  of `OpAppendText`; tuple element reads use `OpArgText` instead of `OpVarText`.

- **Tuple function return + destructuring** (T1.8a) — Functions declared `-> (T1, T2)`
  now work end-to-end: the return value is materialised on the caller's stack, element
  access (`pair(3,7).0`) compiles and executes correctly, and LHS tuple destructuring
  (`(a, b) = pair(5)`) is fully supported.  Two fixes enabled this: the two-zone slot
  allocator now emits a no-op for zone-1 Tuple null-inits (space pre-reserved by
  `OpReserveFrame`) and a per-element push for zone-2 Tuple null-inits; the parser
  now marks destructuring targets as defined and types them on both passes so
  `known_var_or_type` does not fire a false "Unknown variable" on the second pass.

- **`size(t)` character count** — `size("héllo")` returns 5 (Unicode code points),
  complementing `len()` which returns byte length. Backed by a new `OpSizeText` opcode.

- **`FileResult` enum** — Filesystem-mutating operations (`delete`, `move`, `mkdir`,
  `mkdir_all`, `set_file_size`) now return a `FileResult` enum (`Ok`, `NotFound`,
  `PermissionDenied`, `IsDirectory`, `NotDirectory`, `Other`) instead of `boolean`.
  Use `.ok()` for a simple success check.

- **Vector aggregates** — `sum_of`, `min_of`, `max_of` for `vector<integer>`, implemented
  as `reduce` wrappers with internal helper functions. Predicate aggregates `any(vec, pred)`,
  `all(vec, pred)`, `count_if(vec, pred)` with short-circuit evaluation and lambda support.

- **Nested match patterns** — Field positions in struct match arms support sub-patterns:
  `Order { status: Paid, amount } => charge(amount)`. Supports enum variants, scalar
  literals, wildcards, and or-patterns (`Paid | Refunded`).

- **Field iteration** — `for f in s#fields` iterates over a struct's primitive fields
  at compile time. Each iteration provides `f.name` (field name) and `f.value` (a
  `FieldValue` enum wrapping the typed value). Works for uniform and mixed-type structs.

- **Generic functions** — `fn name<T>(x: T) -> T { ... }` declares a generic function.
  T must appear in the first parameter (directly or as `vector<T>`). The compiler creates
  specialised copies per concrete type at each call site (P5.2). Disallowed operations on
  T (arithmetic, field access, methods) produce clear compile-time errors (P5.3).
  Documentation test and LOFT.md section added (P5.4).

- **Shadow call-frame vector** (TR1.1) — The interpreter now tracks a shadow call stack
  with function identity and argument layout on each call/return.  The OpCall bytecode
  format encodes the definition number and argument size.  Foundation for `stack_trace()`.

- **Stack trace types** (TR1.2) — `ArgValue`, `ArgInfo`, `VarInfo`, and `StackFrame` types
  declared in `default/04_stacktrace.loft`.  These will be materialised by `stack_trace()`
  in TR1.3.

- **Closure capture analysis** (A5.1) — Lambdas that reference variables from an enclosing
  scope now produce a clear error: "lambda captures variable 'name' — closure capture is
  not yet supported, pass it as a parameter".  Previously this silently created a broken
  local variable.

- **Closure record layout** (A5.2) — For each capturing lambda, the parser now synthesizes
  an anonymous struct type (`__closure_N`) whose fields match the captured variables'
  names and types.  The record def_nr is stored on the lambda's Definition.

- **`stack_trace()` function** (TR1.3) — Returns `vector<StackFrame>` with function name,
  file, and call-site line for each active call frame.  Arguments/variables vectors are
  left empty (full population is future work).  Implemented as a native function with
  call-stack snapshot bridging State to Stores.

- **Call-site line numbers** (TR1.4) — `CallFrame` now stores the source line directly,
  resolved from `line_numbers` at call time.  Eliminates the per-frame HashMap lookup
  during stack trace materialisation.

- **Coroutine types** (CO1.1) — `CoroutineStatus` enum (Created, Suspended, Running,
  Exhausted) declared in `default/05_coroutine.loft`.  `CoroutineFrame` struct and
  coroutine storage infrastructure added to State.

- **`init(expr)` field initialiser** (L7) — `init(expr)` field modifier evaluates once
  at record creation (with `$` access), stores the result, and allows mutation afterward.
  Complements `computed(expr)` (read-only, recomputed on every access).

- **Tuple type system** (T1.1) — `Type::Tuple(Vec<Type>)` variant added to the type
  enum.  Helper functions `element_size`, `element_offsets`, and `owned_elements`
  provide reusable layout calculations for tuples and closure records.

- **Tuple parser** (T1.2) — Tuple type notation `(T1, T2)` is recognized in all type
  positions.  Tuple literals `(expr, expr)`, element access `t.0`, and LHS
  destructuring `(a, b) = expr` are parsed.  `Value::Tuple` IR variant added.

- **Tuple scope analysis** (T1.3) — Scope analysis recognizes `Type::Tuple` variables
  and identifies owned elements for reverse-order cleanup on scope exit.

- **Closure capture diagnostic** (A5.3) — The closure capture error message now
  indicates that closure body reads (A5.4) are the remaining blocker.  The closure
  record struct from A5.2 is still synthesized.

- **Tuple bytecode codegen** (T1.4) — `Value::TupleGet(var, idx)` IR variant for
  element reads.  Codegen emits `OpVar*` at the element's stack offset.  Tuple
  literals, element access, type annotations, and parameters now work end-to-end.

- **Closure body reads** (A5.4) — Captured variable reads inside lambdas now redirect
  to field loads from a hidden `__closure` parameter backed by the A5.2 closure record
  struct.  Read-only captures work; mutable captures are pending.

- **Coroutine opcodes** (CO1.2) — `OpCoroutineCreate` and `OpCoroutineNext` opcodes
  implemented.  Create copies arguments into a `CoroutineFrame` without entering the
  body.  Next restores the frame's stack and resumes execution.

- **`OpCoroutineReturn`** (CO1.3a) — Opcode to exhaust a running coroutine: clears
  frame state, pushes null, returns to consumer.

- **`OpCoroutineYield`** (CO1.3b) — Opcode to suspend a generator: serialises the
  live stack to `stack_bytes`, saves call frames, slides the yielded value to the
  frame base, and returns to the consumer.  Integer-only path; text serialisation
  pending (CO1.3d).

- **`yield` keyword** (CO1.3c) — Parser recognises `yield expr` in generator
  functions (return type `iterator<T>`).  Codegen emits `OpCoroutineCreate` for
  generator calls, `OpCoroutineYield` for yield statements, and `OpCoroutineReturn`
  at generator body end.  `iterator<T>` single-parameter syntax now accepted.

- **Generator type fixes** (CO1.3c-fix) — Generator body return-type check
  suppressed.  `next(gen)` and `exhausted(gen)` wired as special dispatch calls.
  Coroutine iterators no longer materialised into vectors.  `Type::Iterator` sized
  as DbRef.  `coroutine_create_basic` and `coroutine_next_sequence` tests pass.

- **Closure lifetime** (A5.5) — Closure record work variable is already freed by
  existing `OpFreeRef` scope-exit logic.  No new code needed.

- **`exhausted()` stdlib** (CO1.6) — `OpCoroutineExhausted` opcode and `pub fn
  exhausted(gen) -> boolean` declared in `05_coroutine.loft`.

- **`next()` stack tracking fix** (CO1.6a) — `OpCoroutineNext` and
  `OpCoroutineExhausted` now bypass the operator codegen path.  Stack position
  manually adjusted for DbRef consumption and value push.

- **Null sentinel on exhaustion** (CO1.6c) — `coroutine_next` pushes `i32::MIN`
  (integer null) when the generator is exhausted, not uninitialized bytes.

- **For-loop over generators** (CO1.5a+b) — `for n in gen() { ... }` works.
  The iterator protocol detects generator calls, stores the DbRef in a `__gen`
  variable, and uses `OpCoroutineNext` as the advance step with null-check
  termination.  All 6 coroutine tests pass.

- **`e#remove` rejection** (CO1.5c) — `#remove` on a generator for-loop variable
  produces a compile error (existing guard; coroutine loops never call `set_loop`).

- **Nested yield verified** (CO1.3e) — Generator calling a helper function between
  yields correctly saves/restores call frames across yield/resume.

- **`yield from` parsing** (CO1.4) — `yield from sub_gen` desugars to a loop that
  advances the sub-generator and forwards each value via `yield`.  Test `#[ignore]`
  pending slot-assignment fix.

- **Closure call-site allocation** (A5.3) — Capturing lambdas now allocate the
  closure record on the heap, populate fields from captured variables, and inject
  the record as a hidden argument at call sites.  Multi-capture variable redirect
  fixed (pre-has_var check).  Blocked by slot-assignment issue at codegen time.

- **Tuple element assignment** (T1.4) — `t.0 = expr` now works via `Value::TuplePut`
  IR variant.  Parser detects `TupleGet` on the LHS of `=` and routes through
  element-write codegen.

- **Reference-tuple parameters** (T1.5) — A `RefVar(Tuple)` parameter can now have
  its elements read and written using `.0`, `.1` … notation.  Codegen emits
  `OpVarRef` plus element `OpGet*`/`OpSet*` at the correct byte offset.

- **Unused-mutation guard for tuple refs** (T1.6) — Passing a tuple by reference to
  a function that never writes its elements now produces a WARNING (not an error),
  consistent with the existing scalar-ref mutation guard.

- **`integer not null` annotation** (T1.7) — `Type::Integer` gains a third boolean
  field (`not_null`).  The parser accepts the `not null` suffix on integer type names.
  Assigning a nullable value to a `not null` element in a tuple literal is a
  compile-time error.

- **Text parameter survives coroutine yield** (CO1.3d) — Two root causes for SIGSEGV
  in generators that hold a `text` parameter across `yield`:
  (1) `coroutine_create` now appends the 4-byte return-address slot to `stack_bytes`
  so that `get_var` offsets match the codegen-time layout on every resume;
  (2) `Value::Yield` codegen now decrements `stack.position` by the yielded value's
  size after emitting `OpCoroutineYield`, so subsequent variable accesses in the same
  generator use correct offsets on the second and later resumes.

### Bug fixes

- **Fix #87** — `static_call` no longer snapshots the call stack on every native
  function call; the snapshot now only runs when `n_stack_trace` is dispatched.

- **Fix #88** — `stack_trace()` now includes the entry function (main/test) as the
  outermost frame.

- **Null-coalescing fix** — `f() ?? default` no longer calls `f()` twice; non-trivial
  LHS expressions are materialised into a temporary before the null check.

- **Format specifier warnings** — Compile-time warnings for format specifiers that
  have no effect: hex/binary/octal on text or boolean, zero-padding on text.

- **Slot bug S17: text below TOS in nested scopes** — The two-zone slot redesign
  (0.8.3) fixed the `[generate_set]` panic for text variables pre-assigned below
  the actual TOS in deeply nested scopes.  `text_below_tos_nested_loops` passes;
  `#[ignore]` removed.  CAVEATS.md C4 closed.

- **Slot bug S18: sequential file blocks conflict** — Same two-zone redesign fixed
  the `validate_slots` panic from ref-variable slot override in sequential file
  blocks.  `sequential_file_blocks_read_conflict` passes; `#[ignore]` removed.
  CAVEATS.md C5 closed.

- **`while` loop** (L10) — `while cond { body }` is now a first-class keyword.
  Desugars to a loop with an `if !cond { break }` guard at the top, identical to
  the `for + break` workaround but with familiar syntax.  C11 closed.

### Language changes

- **Format specifier mismatches are now errors** (L9) — Using a radix specifier
  (`:x`, `:b`, `:o`) on a `text` or `boolean` value, or zero-padding (`:05`) on a
  `text` value, is now a compile error rather than a silent no-op.  C14 closed.

### Bug fixes

- **S15: match arm binding type reuse** — When multiple struct-enum match arms bind the
  same field name with different types, each arm now gets its own variable. Previously
  the second arm reused the first arm's type, causing garbled values.

- **S14: stdlib struct-enum field positions** — Struct-enum types defined in the default
  library (`FieldValue`, etc.) no longer panic with "Fld N is outside of record". Fixed
  two issues in `typedef.rs`: loop range for `fill_all()` and lazy byte-type registration.

---

## [0.8.3] — 2026-03-27

### New features

- **WASM output capture** (W1.2) — `output_push` / `output_take` helpers buffer `println`
  output in a thread-local string.  Used by `compile_and_run()` to collect program output
  without touching the filesystem.

- **WASM `compile_and_run()` entry point** (W1.9) — A `compile_and_run(files_json) -> String`
  function accepts a JSON array of `{name, content}` objects, runs the loft pipeline entirely
  in memory, and returns `{output, diagnostics, success}` JSON.  Exported via `wasm_bindgen`
  when built with `--features wasm`.  Default standard library files are embedded with
  `include_str!()`.  A virtual filesystem (`VIRT_FS`) routes `use` imports to the supplied
  in-memory files.

- **`#native "symbol"` annotation** (A7.1) — Functions declared in loft can carry a
  `#native "symbol_name"` annotation.  When the compiler resolves such a function it emits
  an `OpStaticCall` pointing to `symbol_name` in the native registry instead of the loft
  function name.  This decouples the loft identifier from the Rust symbol.

- **Native extension loader** (A7.2) — The `native-extensions` Cargo feature enables
  loading cdylib shared libraries at runtime via `libloading`.  `extensions::load_all()`
  is called between byte-code generation and execution; each library must export a
  C-ABI `loft_register_v1(*mut LoftPluginCtx)` entry point.

- **`LoftPluginCtx` public ABI** (A7.3) — `LoftPluginCtx` is a stable `repr(C)` struct
  published from `loft::extensions` and mirrored in the standalone `loft-plugin-api` crate.
  Plugin crates call `ctx.register_fn(name, fn_ptr)` once per exported function.

- **Format-string buffer pre-allocation** (O7) — The native/WASM code generator now emits
  `String::with_capacity(N × 8)` instead of `"".to_string()` at the start of format strings
  with ≥ 2 segments.  This avoids repeated `String` reallocations during format-string
  assembly, reducing the wasm/native performance gap on string-heavy workloads.

- **VirtFS JavaScript class** (W1.10) — `tests/wasm/virt-fs.mjs` provides a full in-memory
  virtual filesystem for WASM Node.js tests.  Features: tree-based JSON representation
  (`$type`/`$content` conventions), base64 binary support, path normalisation (`.`/`..`/`//`),
  `snapshot()`/`restore()` for test isolation, binary cursors (`seek`/`readBytes`/`writeBytes`),
  `toJSON()`/`fromJSON()` serialisation, and a minimal test harness (`harness.mjs`).
  13 unit tests in `virt-fs.test.mjs` cover all operations.  Runs via
  `node tests/wasm/virt-fs.test.mjs` when Node.js is available.

- **WASM test suite runner** (W1.13) — `tests/wasm/suite.mjs` discovers all loft programs
  in `tests/scripts/` and `tests/docs/`, runs each through the WASM module with a
  pre-populated VirtFS, and compares output against the native `cargo run` interpreter.
  Skips non-deterministic tests (time, unseeded random, images); verifies WASM success only
  for those.  Run via `node tests/wasm/suite.mjs` after building with `wasm-pack`.
  This is the main confidence gate for the WASM port.

- **LayeredFS class** (W1.12) — `tests/wasm/layered-fs.mjs` implements a two-layer virtual
  filesystem: an immutable base tree (bundled examples/docs/stdlib) plus a mutable delta
  overlay (user edits, persisted to localStorage).  Reads check delta first then fall through
  to base; writes always go to delta, leaving the base untouched.  Supports
  `getDelta()`/`setDelta()`/`saveDelta()`/`resetToBase()`/`isModified()`/`isDeleted()`.
  `ide/scripts/build-base-fs.js` reads `tests/docs/*.loft`, `doc/*.html`, and
  `default/*.loft` to emit `ide/assets/base-fs.json`.  20 unit tests in
  `layered-fs.test.mjs` cover all operations including delta serialisation and snapshot
  isolation.

- **loftHost factory** (W1.11) — `tests/wasm/host.mjs` exports `createHost(tree, options)`
  which wires a `VirtFS` instance to the full `loftHost` bridge API.  Uses a deterministic
  xoshiro128** PRNG for reproducible `rand()` / `rand_seed()` behaviour in tests.  Supports
  configurable `fakeTime`, `fakeTicks`, `env`, and `args` overrides.  Comes with:
  `bridge.test.mjs` (7 WASM integration tests; skips gracefully when `pkg/` not built),
  `file-io.test.mjs` (14 host-level edge-case tests, no WASM required),
  `random.test.mjs` (host PRNG tests + optional WASM-level determinism tests),
  and three fixtures in `tests/wasm/fixtures/`.

---

## [0.8.2] — 2026-03-24

### New features

- **Lambda expressions** — Write inline functions with `fn(x: integer) -> integer { x * 2 }`
  or the short form `|x| { x * 2 }`. Parameter and return types are inferred when the
  context makes them clear (e.g. inside `map`, `filter`, `reduce`). Lambdas cannot capture
  variables from the surrounding scope yet — pass needed values as arguments.

- **Named arguments and defaults** — Functions can declare default values
  (`fn connect(host: text, port: integer = 80, tls: boolean = true)`). Callers can skip
  middle parameters by name: `connect("localhost", tls: false)`.

- **Native compilation** — `loft --native file.loft` compiles your program to a native
  binary via `rustc` and runs it. `loft --native-emit out.rs` saves the generated Rust
  source. `loft --native-wasm out.wasm` compiles to WebAssembly.

- **JSON support** — Serialise any struct to JSON with `"{value:j}"`. Parse JSON into a
  struct with `Type.parse(json_text)` or into an array with `vector<T>.parse(json_text)`.
  Check for parse errors with `value#errors`.

- **Computed fields** — Struct fields marked `computed(expr)` are recalculated on every
  read and take no storage: `area: float computed(PI * $.r * $.r)`.

- **Field constraints** — Struct fields can declare runtime validation:
  `lo: integer assert($.lo <= $.hi)`. Constraints fire on every field write.

- **Parallel workers now support text and enum returns** — `par(...)` workers can return
  `text` and inline enum values in addition to the existing `integer`, `long`, `float`,
  and `boolean`. Workers can also receive extra context arguments beyond the loop element.

### Language changes

- **Function references drop the `fn` prefix** — Write `apply(double, 7)` instead of
  `apply(fn double, 7)`. Using `fn name` as a value is now a compile error.

- **Short-form lambdas infer types** — `|x| { x * 2 }` infers parameter and return
  types from the call site. Use the long form `fn(x: integer) -> integer { ... }` when
  you need explicit types.

- **Private by default** — Definitions without `pub` are no longer visible to `use`
  imports from other files.

### Better error messages

- Using `string` as a type now suggests `text` instead of a generic error.
- Match exhaustiveness errors now point at the `match` keyword, not the closing brace.
- Six common errors now include fix suggestions (e.g. "use a new variable name or
  cast with 'as'" for type-change errors).
- Three errors that previously stopped all parsing now let the compiler continue and
  report additional issues.
- Several places that crashed the compiler on unusual input now produce a proper error.

### Bug fixes

- `c + d` where both are characters no longer crashes. The result is text concatenation.
- PNG image loading now reports correct `width` and `height` values.
- Passing an empty vector `[]` directly as a function argument no longer crashes.
- `v += other_vec` on vectors containing text fields no longer corrupts the original.
- `&vector` parameters correctly propagate appends back to the caller.
- Vector slices assigned to a variable (`s = v[1..3]`) are now independent copies.
- `map`, `filter`, and `reduce` no longer cause internal slot conflicts.

---

## [0.8.0] — 2026-03-17

### New features

- **Match expressions** — Pattern match on enums, structs, and scalar values:
  ```loft
  match shape {
      Circle { r } => PI * pow(r, 2.0),
      Rect { w, h } => w * h,
  }
  ```
  The compiler checks that all variants are handled. Supports or-patterns
  (`North | South =>`), guard clauses (`if r > 0.0`), range patterns (`1..=9`),
  null patterns, character patterns, and block bodies.

- **Code formatter** — `loft --format file.loft` formats a file in-place.
  `loft --format-check file.loft` exits with an error if the file is not formatted.

- **Wildcard and selective imports** — `use mylib::*` imports everything;
  `use mylib::Point, add` imports only specific names. Local definitions take priority
  over imports.

- **Callable function references** — Store a function in a variable and call it:
  `f = fn double; f(5)`. Function-typed parameters also work.

- **`map`, `filter`, `reduce`** — Higher-order collection functions that accept
  function references: `map(numbers, fn double)`.

- **Test runner improvements** — `loft --tests file.loft::test_name` runs a single test.
  `loft --tests 'file.loft::{a,b}'` runs multiple. `loft --tests --native` compiles
  tests to native code first.

- **`now()` and `ticks()`** — `now()` returns milliseconds since the Unix epoch.
  `ticks()` returns microseconds since program start (monotonic timer).

- **`mkdir(path)` and `mkdir_all(path)`** — Create directories from loft code.

- **`vector.clear()`** — Remove all elements from a vector.

- **External library packages** — `use mylib;` can now resolve packaged library
  directories with a `loft.toml` manifest file.

### Diagnostics

- Warning for division or modulo by constant zero.
- Warning for unused loop variables (suppress with `_` prefix: `for _i in ...`).
- Warning for unreachable code after `return`, `break`, or `continue`.
- Warning for redundant null checks on `not null` fields.
- Warning when not all code paths return a value in a `not null` function.

### Bug fixes

- `x << 0` and `x >> 0` now correctly return `x` instead of null.
- `NaN != x` now returns `true` (was incorrectly `false`).
- `??` (null coalescing) on float values works correctly.
- Using `if` as a value expression without `else` is now a compile error instead of
  silently producing null.
- Assigning `null` to a struct field no longer causes a runtime crash.
- Functions with multiple owned struct variables no longer crash on cleanup.
- `sorted[key] = null` and `hash[key] = null` removal works again (was broken by a
  null-handling fix).
- `v += other_vec` on vectors with text fields no longer corrupts data.
- `index<T>` fields inside structs can now be copied and reassigned.
- Sorted filtered loop-remove, index key-null removal, and index loop-remove all fixed.
- `??` null coalescing, non-zero exit on errors, reverse iteration on `sorted<T>`,
  CLI args in `fn main`, format specifier sign order, XOR/OR/AND with null values,
  and `for c in enum_vector` infinite loop — all fixed.

---

## [0.1.0] — 2026-03-15

First release.

### Language

- **Static types with inference** — Types are checked at compile time. No annotations
  needed; the type is inferred from the first assignment.
- **Null safety** — Every value is nullable unless declared `not null`. Null propagates
  through arithmetic. Use `?? default` to provide a fallback value.
- **Primitive types** — `boolean`, `integer`, `long`, `float`, `single`, `character`, `text`.
- **Structs** — Named records with fields: `Point { x: 1.0, y: 2.0 }`.
- **Enums** — Plain enums (named values) and struct-enums (variants with different fields
  and per-variant method dispatch).
- **Control flow** — `if`/`else`, `for`/`in`, `break`, `continue`, `return`.
- **For-loop extras** — Inline filter (`for x in v if x > 0`), loop attributes
  (`x#first`, `x#count`, `x#index`), in-loop removal (`v#remove`).
- **Vector comprehensions** — `[for x in v { expr }]`.
- **String interpolation** — `"Hello {name}, score: {score:.2}"` with format specifiers.
- **Parallel execution** — `for a in items par(b=worker(a), 4) { ... }` runs work across
  CPU cores.
- **Collections** — `vector<T>` (dynamic array), `sorted<T>` (ordered tree),
  `index<T>` (multi-key tree), `hash<T>` (hash table).
- **File I/O** — Read, write, seek, directory listing, PNG image support.
- **Logging** — `log_info`, `log_warn`, `log_error` with source location and rate limiting.
- **Libraries** — `use mylib;` imports from `.loft` files.

---

[0.8.3]: https://github.com/jjstwerff/loft/compare/v0.8.2...v0.8.3
[0.8.2]: https://github.com/jjstwerff/loft/compare/v0.8.0...v0.8.2
[0.8.0]: https://github.com/jjstwerff/loft/compare/v0.1.0...v0.8.0
[0.1.0]: https://github.com/jjstwerff/loft/releases/tag/v0.1.0
