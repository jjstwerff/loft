# Changelog

All notable changes to the loft language and interpreter.

---

## [Unreleased]

### Parallel execution

- **`par_light` runtime foundation** (A14.1–A14.4):
  - A14.1: `Store::borrow_locked_for_light_worker` — O(1) read-only view sharing the
    original's buffer pointer. `borrowed` field prevents double-free on Drop.
  - A14.2: `WorkerPool` — pre-allocates `n_workers × M` stores, reused across invocations.
  - A14.3: `Stores::clone_for_light_worker` — assembles worker view with shallow borrows
    of main stores + fresh pool stores. Zero large buffer copies.
  - A14.4: `run_parallel_light` — drop-in for `run_parallel_direct` using the pool.
  - A14.5: `check_light_eligible` — DFS call-graph analysis validates no recursive store
    allocation. Returns `M` (pool stores per worker) for eligible workers.
  - A14.6: `build_parallel_for_ir` automatically selects `n_parallel_for_light` when
    the worker qualifies (primitive return, no recursive allocation). No new syntax —
    `par(...)` is transparently optimized.
  - A14.7: `n_parallel_for_light` native function registered. Allocates result vector,
    creates `WorkerPool`, dispatches via `run_parallel_light`.
  Auto-selection is fully enabled: eligible `par()` workers (primitive return,
  no recursive store allocation) transparently use the light path.  Three bugs
  fixed in the enablement: stack pop order in the native function, result DbRef
  `pos` field (4 not 8), and store borrow range (all stores, not just `[..max]`).

### Sorted collection slicing (A8)

- **Partial-key match iterator** (A8.3): `idx[k1]` on a multi-key index now iterates
  all elements matching the first key. Parser detects `nr < key_types.len()` and emits
  an inclusive range with `from = till = [k1]`. The existing `key_compare` zip-based
  comparison treats partial prefixes as unconstrained on remaining fields.

### WASM parallel infrastructure (W1.18)

- **WASM Worker Thread infrastructure** (W1.18-1 through W1.18-5):
  - W1.18-1: `#[cfg(all(feature = "wasm", feature = "threading"))]` branch in
    `run_parallel_direct` dispatches to JS host via `parallel_run()`.
  - W1.18-2: `worker_entry(fn_index, start, end)` exported via `#[wasm_bindgen]`.
  - W1.18-3: `tests/wasm/worker.mjs` — Worker Thread park/wake loop.
  - W1.18-4: `tests/wasm/parallel.mjs` — `LoftThreadPool` class.
  - W1.18-5: `tests/wasm/harness.mjs` — `initThreaded()` for shared-memory WASM.
  W1.18-6 (test enablement) deferred until wasm-threads build is available.

### Closures (A5.6-text, C30)

- **Cross-scope text-capturing closures** — `make_greeter("Hello")("world")` now
  produces `"Hello world"`.  Three bugs fixed:
  - `can_convert` now handles `Function` types recursively and treats `Text` types
    with different dependency lists as compatible.
  - Chained fn-ref calls always allocate 1 work-buffer for text returns, keeping
    variable creation identical on both parser passes (fixes SIGSEGV from counter
    desynchronization).
  - Cross-scope closures: `skip_free` on the closure work-var when the enclosing
    function returns `Type::Function`; `Value::FreeFnRefClosure` IR node frees the
    closure in the caller's chained-call block.
  - `Variable.captured` flag suppresses false "never read" warning for captured
    parameters without affecting dead-assignment analysis.
- **Lambda re-definition no longer leaks or crashes** (C30) — reassigning a variable
  that holds a capturing lambda (`f = fn(y) {...}; f = fn(y) {...}`) previously
  created two closure work-vars that both owned the same store, causing a SIGSEGV in
  debug builds and a store leak in release.  Fix: reuse the existing closure work-var
  on reassignment (`skip_free` for all capturing closure work-vars; emit
  `FreeFnRefClosure` before each `SetVar` on a `Function`-typed variable).  Also fixed
  a SIGSEGV in the debug logger that crashed when printing `FreeFnRefClosure` and other
  fn-ref opcodes.  Test: `closure_redefine_frees_old` (previously `#[ignore]`).
- **`element_store_size` corrected for `Type::Function`** (C31 groundwork) —
  previously returned 12 (falling through to the reference-sized arm); now returns
  16 (4 B definition number + 12 B closure DbRef).  A `debug_assert!` guard on the
  default arm catches any future unhandled type.  The full C31 fix (storing closures
  in vectors) remains open.

### Debugging infrastructure

- **Debug boundary checks for DbRef, record fields, and stack pops** —
  Five `debug_assert!` additions (zero cost in release builds):
  - `keys::store()` / `keys::mut_store()`: assert `store_nr < allocations.len()` with
    clear message showing both values.
  - `Store::addr()` / `Store::addr_mut()`: validate field offset against the record's
    claimed size (first word of record header). Fires for `rec > 1, fld > 0`.
  - `Stores::get<T>()`: assert `stack.pos >= size_of::<T>()` before decrement, catching
    stack underflow from wrong native-function pop order.
  - `state::put_var`: assert slot index is within stack bounds before writing a
    variable slot (catches off-by-one slot assignment errors).
  - `collections::element_store_size`: assert all types are handled explicitly;
    panics with a clear message if a new type is added without a corresponding arm.

### Safety fixes

- **Coroutine store-mutation guard promoted to always-on** (CO1.9) — The generation
  counter in `Store` and the `saved_store_generations` snapshot in `CoroutineFrame`
  were previously compiled only under `#[cfg(debug_assertions)]`.  All `#[cfg]` gates
  have been removed so the guard fires in release builds too.  `debug_assert!` in
  `coroutine_next` is replaced with `assert!`, meaning a mutated-store violation now
  panics with a clear diagnostic in any build profile:
  `"stale DbRef: store N was mutated between coroutine yields (generation at yield: X,
  now: Y) — DbRef locals held by the generator may point to freed or reallocated records"`.
  The affected sites in `store.rs` are `claim`, `resize`, `delete`, and the two
  `clone_locked*` constructors.  New test `coroutine_stale_store_guard_all_builds`
  (no `#[cfg(debug_assertions)]` gate) confirms the panic fires unconditionally.

### Language features

- **`interface` keyword and first-pass parser** (I1, I2, I3) — The first three steps
  of the interface subsystem are implemented:
  - I1 (`src/lexer.rs`): `"interface"` is now a reserved keyword.
  - I2 (`src/data.rs`): `DefType::Interface` added to the definition-type enum;
    `Definition.bounds: Vec<u32>` added to hold interface constraints for bounded
    generic functions (`<T: A + B>`); initialised to `vec![]` in `add_def`.
  - I3 (`src/parser/definitions.rs`, `src/parser/mod.rs`): new `parse_interface()`
    method parses `interface Name { fn method(params) -> type }` declarations.
    `Self` is temporarily registered as a type placeholder so `parse_type_full`
    resolves it during method signature parsing.  Duplicate interface names emit
    "Redefined interface Name".  `parse_interface` is added to the `parse_file`
    top-level dispatch chain alongside `parse_struct`, `parse_enum`, etc.
  Tests: `interface_empty_parses`, `interface_with_method_parses`,
  `interface_duplicate_name_rejected`.

- **Interface subsystem — op-sugar, bound syntax, factory-method guard, gendoc skip** (I3.1, I4, I5, I11):
  - I3.1 (`src/parser/definitions.rs`): `op <token> (params) -> type` in interface bodies
    is syntactic sugar for an `OpCamelCase` method stub. E.g. `op < (self: Self, other: Self) -> boolean`
    registers a method named `OpLt`. The `rename()` helper in `mod.rs` is now `pub(crate)` and
    covers `>` and `>=` in addition to its previous set.
    Tests: `interface_op_sugar_lt_parses`, `interface_op_sugar_multi_parses`.
  - I4 (`src/parser/definitions.rs`): `<T: A + B>` bound syntax in generic function declarations.
    Bound names are collected during parsing and resolved in the second pass to `DefType::Interface`
    def_nrs stored in `Definition.bounds` (introduced in I2). Unknown names emit
    `"'Name' is not a known interface"`; non-interface names emit
    `"'Name' is not an interface — bounds must be interface names"`.
    Tests: `generic_fn_with_bound_parses`, `generic_fn_unknown_bound_errors`,
    `generic_fn_struct_as_bound_errors`.
  - I5 (`src/parser/definitions.rs`): phase-1 factory-method restriction in interface bodies.
    A method that returns `Self` without a leading `self: Self` parameter emits
    `"factory methods not yet supported: 'name' returns Self without a 'self: Self' parameter"`.
    Test: `interface_factory_method_rejected`.
  - I11 (`src/gendoc.rs`): `sig_kind` now returns `"interface"` for `pub interface` / `interface`
    declarations (previously `"const"`). `generate_stdlib_section` skips interface items gracefully.
    Unit test: `sig_kind_interface_returns_interface`.

- **Interface subsystem — satisfaction checking, bounded method/operator calls** (I6, I7, I8.1, I10):
  - I6 (`src/parser/mod.rs`): `check_satisfaction` verifies that a concrete type implements
    every method declared in a bounded generic's interface constraints. Called from
    `try_generic_instantiation` — emits `"'Type' does not satisfy interface 'Name': missing Method"`.
    Tests: `satisfaction_check_passes_with_implementing_type`,
    `satisfaction_check_fails_missing_method`.
  - I7 (`src/parser/fields.rs`, `src/parser/definitions.rs`): T-parameterized method stubs
    (e.g. `t_1T_label`) are created during second-pass bounds resolution. `field()` looks up
    the T-stub via `find_fn` before reporting "field access requires a concrete type", enabling
    `v.method()` inside generic bodies. `re_resolve_call` substitutes the concrete implementation
    at specialization time.
    Test: `bounded_method_call_in_generic_body`.
  - I8.1 (`src/parser/mod.rs`): `call_op` looks up T-stubs for operators (e.g. `t_1T_OpLt`)
    before erroring, enabling `a < b` inside bounded generic bodies. First-pass operator calls
    on T now return `Type::Void` instead of erroring, allowing the second pass to proceed.
    Test: `bounded_operator_in_generic_body`.
  - I10: satisfaction diagnostics share the I6 implementation above.
  - Supporting changes: `Data::children_of` iterates definitions by parent;
    `field()` returns `Type::Unknown(0)` in the first pass for unknown-type field access
    (previously errored); user-defined operator functions (e.g. `fn OpLt(self: Score, ...)`)
    are now allowed in user code without a lowercase name error.

- **Interface operator variants and stdlib `Ordered`** (I8.2, I8.3, I8.4, I9):
  - I8.2: Return-type propagation from interface signature — verified: T-stubs correctly
    substitute `Self` → `T` in both parameter types and the return type.
    Test: `bounded_operator_self_return_type`.
  - I8.3: Mixed-type binary operators (`T op concrete`, e.g. `T * integer`) — verified:
    `call_op`'s T-stub lookup and `call_nr`'s argument matching handle mixed-type parameters.
    Test: `bounded_mixed_type_operator`.
  - I8.4: Unary operators on `T` (e.g. `op -`) — verified: single-operand dispatch uses the
    same `call_op` → T-stub path as binary operators.
    Test: `bounded_unary_operator`.
  - I9: `pub interface Ordered { op < }` added to `default/01_code.loft`. User types satisfy
    `Ordered` by defining `fn OpLt(self: T, other: T) -> boolean`. Existing tests updated to
    use the stdlib interface instead of local redefinitions.
    Test: `stdlib_ordered_interface`.

- **Built-in type satisfaction and stdlib Equatable/Addable** (I9-prim, I9-Eq, I9-Add, I9.1):
  - I9-prim: `find_fn` now falls back to the `possible` operator map when the method-style
    name (`t_7integer_OpLt`) is not found. This lets built-in types (integer, float, etc.)
    satisfy interfaces since their operators use the `add_op` convention (`OpLtInt`).
    `call_op` skips the main operator loop when an operand is a generic type variable,
    preventing false matches via `OpEqRef` / `OpEqBool` implicit conversions.
    `check_satisfaction` delegates to `find_fn` for both naming conventions.
    Tests: `builtin_integer_satisfies_ordered`, `builtin_float_satisfies_ordered`.
  - I9-Eq: `pub interface Equatable { op == }` added to `default/01_code.loft`.
    Test: `stdlib_equatable_interface`.
  - I9-Add: `pub interface Addable { op + }` added to `default/01_code.loft`.
    Test: `stdlib_addable_interface`.
  - I9.1: bounded generics with Addable work on integer and float types.
    Tests: `generic_sum_pair_on_integers`, `generic_sum_pair_on_floats`.

- **Vector<T> element access fix and Numeric interface** (I9-vec, I9.1, I9.2, I9+):
  - I9-vec: fix vector element access in generic specialization. `substitute_type_in_value`
    detects `OpGetVector` calls with baked-in `elm_size=0` (from type variable elements),
    recomputes the correct size from the concrete type, and adds the value-extraction wrapper
    (`OpGetInt`/`OpGetFloat`/etc.).  First-pass `call_op` for generic types now returns the
    type variable type (not `Type::Void`) to prevent "cannot change type" errors.
    Test: `generic_vector_element_access`.
  - I9.1: bounded-generic comparison on vector elements using `Ordered` bound.
    Test: `generic_min_of_vector_elements`.
  - I9.2: bounded-generic sum of vector elements using `Addable` bound.
    Test: `generic_sum_on_integer_vector`.
  - I9+: `pub interface Numeric { op * ; op - }` added to `default/01_code.loft`.
    Test: `stdlib_numeric_interface`.

- **Generic accumulator fix, Scalable interface** (I9-var, I9.1, I9.2, I9-Sc):
  - I9-var: skip `ref_return`/`text_return` for generic templates (`DefType::Generic`).
    The return type `T = Reference(tv_nr)` triggered `ref_return` which promoted local
    variables to hidden parameters.  After specialization to Integer/Float, the hidden
    params caused a codegen crash.  This enables for-loop accumulator patterns inside
    generic bodies.
    Tests: `generic_intermediate_variable`, `generic_for_loop_accumulator`.
  - I9.1: generic `find_max` on integer vectors using `Ordered` for-loop accumulator.
    Test: `generic_max_on_integer_vector`.
  - I9.2: generic `vec_sum` with caller-supplied identity using `Addable` for-loop.
    Test: `generic_sum_with_identity`.
  - I9-Sc: `pub interface Scalable { fn scale(self, factor: integer) -> integer }` in
    `default/01_code.loft`.  Uses a method (not `op *`) to avoid stub-name collision
    with `Numeric`.
    Test: `stdlib_scalable_interface`.

- **Interface stub collision fix, generic min_of/max_of/sum** (I9-stub, I9.1, I9.2):
  - I9-stub: interface method stubs now use `__iface_{d_nr}_{method}` naming instead of
    `t_4Self_{method}`. Multiple interfaces can now declare the same operator without
    collision. `has_bound_for_method` prevents T-stubs from leaking into unbound generics.
  - I9.1: `min_of` and `max_of` replaced with bounded-generic versions using `Ordered`.
    Now work on integer, float, and any user type satisfying `Ordered`. Unused helper
    functions (`__min_int`, `__min_float`, `__max_int`, `__max_float`) removed.
    Tests: `stdlib_min_of_generic`, `stdlib_max_of_generic`, `stdlib_min_of_float`,
    `stdlib_max_of_float`.
  - I9.2: `pub fn sum<T: Addable>(v: vector<T>, init: T) -> T` added. The caller supplies
    the identity element. Integer-specific `sum_of(v)` kept for backward compatibility.
    Test: `stdlib_sum_generic`.

- **Text-returning interface methods, Printable, coroutine yield-from-loop** (I9-text, I9-Pr, CO1.7):
  - I9-text: T-stub creation adds hidden `__work_1: RefVar(Text)` parameter for
    text-returning interface methods. Matches the hidden param from `text_return` so
    `re_resolve_call` finds the correct argument count.
    Test: `generic_text_returning_method`.
  - I9-Pr: `pub interface Printable { fn to_text(self: Self) -> text }` added to stdlib.
    Test: `stdlib_printable_interface`.
  - CO1.7 (partial): coroutine yield from range-based and vector for-loops verified.
    Tests: `coroutine_yield_from_range_loop`, `coroutine_yield_from_vector_loop`.

- **CO1.7 complete: coroutine yield from all for-loop types** —
  Fixed character null sentinel bug: `push_null_value(4)` uses `i32::MIN` as the
  sentinel for all 4-byte values, but `op_conv_bool_from_character` only checked for
  `char::from(0)`. The `i32::MIN` sentinel (0x80000000) looked like a valid character,
  causing for-loops over character iterators to infinite-loop. Also fixed UB in
  `var_character` (fill.rs): reading `i32::MIN` directly as `char` is not a valid
  Unicode scalar — now reads as `u32` and converts via `char::from_u32`.
  Tests: `coroutine_yield_from_text_loop`, `coroutine_character_iterator_exhausts`,
  `coroutine_yield_from_struct_vector_loop`, `coroutine_yield_from_field_text_loop`.

- **CO1.8 complete: multi-text coroutine safety** — Verified all three CO1.8 sub-items
  pass without code changes: (a) multiple text parameters serialised correctly,
  (b) text locals after first yield survive resume, (c) text locals in nested blocks
  freed correctly. Tests: `coroutine_multi_text_params`, `coroutine_text_local_after_yield`,
  `coroutine_text_local_nested_block`.

- **fix-tvscope: clear diagnostic for type variable name clash** — Defining `struct T`
  when `T` is a generic type variable (from stdlib generics) now produces
  `"'T' is reserved as a generic type variable"` instead of a confusing
  "Redefined struct" message or a runtime crash.

### Sorted collection slicing (A8)

- **Open-ended bounds, range iteration, comprehensions** (A8.1, A8.2, A8.4, A8.6):
  - A8.1: `col[lo..]`, `col[..hi]`, and `col[..]` now work on sorted collections.
    Parser detects `..` before the first expression (open-start) and missing expression
    after `..` (open-end). Runtime handles empty from/till arrays in OpIterate.
    Tests: `sorted_open_end_range`, `sorted_open_start_range`.
  - A8.2: `sorted[lo..hi]` range iteration verified working. Test: `sorted_range_iteration`.
  - A8.4: `[for e in sorted[lo..hi] { expr }]` comprehensions verified.
    Test: `sorted_range_comprehension`.
  - A8.6: nullable lookup `if !col[k]` verified. Test: `sorted_nullable_lookup`.
  - A8.1-idx: open-ended bounds also work on index collections. Test: `index_open_end_range`.
  - A8.5: `rev(col[lo..hi])` reverse range iteration on sorted collections. Parser sets
    `reverse_iterator` flag before the inner subscript expression so `fill_iter` picks it up.
    Test: `sorted_reverse_range`.

### Coroutine safety documentation

- **Coroutine text arg `Str` serialised at create; pointer-patched on resume** (S25.1, S25.2) —
  `State::coroutine_create` now calls `serialise_text_args` after copying the raw
  argument bytes.  For each text (`Str`) argument that points into a dynamic heap
  allocation (not a static literal in `text_code`), the function clones the string
  data into an owned `String` stored in `frame.text_owned`, then overwrites the
  `Str` bytes in `stack_bytes` to point to the owned buffer.  The owned `String`
  outlives any `OpFreeText` the caller may emit after the create; the `Str` pointer
  is therefore never dangling on the first or any subsequent resume (P2-R1, critical
  use-after-free).
  At `coroutine_next`, each owned String's current buffer address is patched back
  into the cloned `stack_bytes` before the bytes are copied to the live stack
  (M6-b pointer-patch step).
  At `coroutine_return`, the existing `frame.text_owned.clear()` now properly drains
  the owned Strings that were populated by S25.1, freeing their heap allocations via
  Rust RAII instead of leaking them (P2-R2, high memory leak).
  Two new tests `coroutine_text_arg_dynamic_serialised` and
  `coroutine_text_arg_freed_at_return` in `tests/expressions.rs` exercise the create
  → resume → exhaust cycle with a dynamically formatted text argument.

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

- **Abandoned coroutine frame freed on early `for` loop exit** (S37) — When a `for`
  loop breaks before a generator exhausts, `OpFreeRef` calls `free_ref` on the
  coroutine DbRef.  `database.free()` is a no-op for `COROUTINE_STORE`
  (store_nr == u16::MAX), so `text_owned` buffers, `stack_bytes`, and `call_frames`
  in the `CoroutineFrame` were silently leaked on every early-break path.
  Fix: `free_ref` now checks `db.store_nr == COROUTINE_STORE` and calls
  `free_coroutine(db.rec)` explicitly before returning.  Test
  `coroutine_early_break_frame_freed` in `tests/expressions.rs` exercises the
  early-break path and verifies the correct first-yield value is returned.

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

### Language features

- **Tuple destructuring in `match`** (T1.9) — `match` now dispatches on `Type::Tuple`
  subjects.  New `parse_tuple_match` in `src/parser/control.rs` parses comma- or
  semicolon-separated arms with wildcard (`_`), binding-variable, and literal patterns.
  Logical AND for multi-element conditions is built as `v_if(a, b, false)` (there is no
  `OpAnd`).  Tests: `tuple_match_wildcard`, `tuple_match_literal`, `tuple_match_binding`.

- **Homogeneous-type tuple coverage** (T1.10) — Three new tests confirm that same-element-type
  tuples work across common data sources: `tuple_homogeneous_text` (`(text, text)` pair
  from function parameters), `tuple_store_text_fields` (text fields extracted from two
  struct records), and `tuple_from_vector_elements` (`(integer, integer)` from indexed
  vector reads).  `tuple_struct_refs` (two `(Point, Point)` DbRefs) remains ignored
  pending T1.8 lifetime tracking for DbRef tuple slots.

- **Tuple type constraint diagnostics** (T1.11) — Two new compile-time guards:
  (a) `struct Foo { pair: (integer, integer) }` now emits "struct field cannot have a
  tuple type — tuples are stack-only values" at parse time (`parse_field` in
  `definitions.rs` detects `(` via `parse_type_full` before `fill_all` is reached);
  (b) `(a, b) += expr` now emits "compound assignment is not supported for tuple
  destructuring — use (a, b) = expr instead" (`parse_assign` in `expressions.rs` returns
  early in both passes, consuming the operator and RHS to keep the parser state clean).

### Coroutine safety documentation

- **Store-backed `Str` debug guard in `coroutine_yield`** (P2-R5 M10-a) — In
  `#[cfg(debug_assertions)]` builds on 64-bit targets, `coroutine_yield` now
  scans every tracked text local in the generator's `locals_bytes` and warns
  (`eprintln!("[P2-R5] ...")`) if the first 8 bytes (the `Str.ptr` field) fall
  within any live non-stack store allocation.  A store-backed Str in a suspended
  generator dangles if the consumer frees or reuses the backing record before
  the next resume.  The check is a heuristic (cannot cover full pointer
  provenance) but catches the common case of a recently-read text field local.
  No change to correct-program behaviour; the warning is diagnostic only.
  See `COROUTINE.md` CL-2b and `SAFE.md` § P2-R5.

- **Yielded `Str` ownership rule documented** (P2-R10) — `COROUTINE.md` CL-7 records
  the ownership invariant for `text` values produced by `yield`: the value is a
  zero-copy reference into the generator's frame (or `text_owned` buffer once CO1.3d
  lands) and is valid only for the current loop-body iteration.  Consumers that need
  to keep the text beyond one iteration must copy it (`stored = "{value}"`) or pass
  it to a function that calls `set_str`.  No runtime change; documentation only.

- **Text locals survive yield/resume in coroutines** (P2-R3 CO1.3d) — Text
  variables in generator functions are `String` objects (24 B) on the live stack.
  The bitwise copy of the locals region at yield is safe: `String` owns its heap
  buffer and no external code can free that buffer while the generator is suspended.
  The M8-b `debug_assert!` that fired for any text local at yield time has been
  removed; the S27 `text_positions` save/restore is preserved for correctness.
  Additionally, `coroutine_return` and `push_null_value` now push
  `Str::new(STRING_NULL)` (not 16 zero bytes) when an exhausted `iterator<text>`
  generator returns its null sentinel — the zero-pointer `Str` caused a panic in
  `append_text` via `slice::from_raw_parts(0, 0)`.  Test
  `coroutine_text_local_survives_yield` in `tests/expressions.rs` is now active and
  passing.

### Native store safety

- **Locked store cleared on free; `40-par-ref-return.loft` fixed** (S36) —
  `free_named` in `src/database/allocation.rs` now calls `unlock()` on the store
  before marking it free in the bitmap.  The parser auto-inserts
  `n_set_store_lock(stores, param, true)` at the start of functions with `const`
  reference parameters but does not emit the matching unlock before return.  When
  the store was freed while still locked, `find_free_slot` selected the freed slot
  for reuse and `database_named` called `init()` on a locked store, triggering:
  "Write to locked store at rec=1 fld=0".  The bug was invisible in the interpreter
  because `test_runner.rs` creates a fresh `Stores` per test function; in native
  mode all `test_*` functions share one `Stores`, so the leaked lock carried over
  from `test_par_struct_simple` into `test_par_struct_return_single_thread`.
  `40-par-ref-return.loft` now passes in `native_scripts` with 45/45.

### Interpreter fixes

- **`20-binary.loft` double-free fixed** (S34) — When `adjust_first_assignment_slot`
  cannot move a work variable downward (same-scope siblings block the move) and
  Option A fires — forcing the variable to the current TOS, aliasing it with the
  outer `rv` — the variable is now marked `skip_free` at that point.
  `generate_call` suppresses the `OpFreeRef` bytecode for any `skip_free` variable,
  preventing the "Double free store" panic caused by both `rv` and `_read_34` each
  trying to free the same database record at slot 820.  `skip_free` flags set during
  codegen are propagated back to `data.definitions[def_nr].variables` before
  `validate_slots` runs, which now skips slot-overlap pairs where either variable is
  `skip_free`.  The `binary` test (`tests/scripts/20-binary.loft`) no longer has
  `#[ignore]`; `"20-binary.loft"` removed from `ignored_scripts()` in `tests/wrap.rs`.

### WASM / native codegen fixes

- **Native codegen: Insert-return pattern fixed** (S35) — `output_set` in
  `src/generation/dispatch.rs` now detects `Value::Insert` as the RHS of an
  assignment and hoists all-but-last ops as standalone statements before the
  declaration line, emitting only the final expression as the assignment value.
  Previously the inner `Set` ops were emitted inline inside an expression context,
  producing malformed Rust (`let mut var_rv: DbRef = let mut var__read_34: DbRef = …`).
  The same function now also suppresses `OpFreeRef` for variables marked `skip_free`,
  matching the bytecode interpreter fix (S34) and preventing a double-free in the
  native binary.  `"20-binary.loft"` removed from `SCRIPTS_NATIVE_SKIP` in
  `tests/native.rs`; `native_binary_script` test passes without `#[ignore]`.

- **WASM random bridge wired; `rand_indices` shuffles via host bridge** (W1.19) —
  `codegen_runtime::n_rand` previously returned `i32::MIN` (null) when compiled
  without `feature = "random"`, making all `rand(lo, hi)` calls return null in WASM.
  It now delegates to `ops::rand_int`, which already had a WASM fallback calling
  `host_random_int` from `src/wasm.rs`.  A matching WASM `shuffle_ints` fallback
  (feature="wasm", not feature="random") was added to `src/ops.rs`, performing a
  Fisher-Yates shuffle via repeated `host_random_int(0, i)` calls; `n_rand_indices`
  in `codegen_runtime.rs` now enables the shuffle for both the PCG and WASM code
  paths.  `"21-random.loft"` removed from `WASM_SKIP` in `tests/wrap.rs`; the WASM
  compilation test now exercises `rand()`, `rand_seed()`, and `rand_indices()`.

- **WASM time bridge wired to `std::time::SystemTime`** (W1.20) — `host_time_now()`
  and `host_time_ticks()` in `src/wasm.rs` previously returned hard-coded `0`.
  They now call `std::time::SystemTime::now()` via the WASI clock interface (available
  in `wasm32-wasip2` through Rust's std).  `host_time_ticks()` delegates to
  `host_time_now()` (millisecond wall-clock); `n_ticks` computes elapsed microseconds
  as `(host_time_ticks() - start_time_ms) * 1000`, which is sufficient for benchmark
  timing.  `"22-time.loft"` removed from `WASM_SKIP` in `tests/wrap.rs`; the WASM
  compilation test now exercises `now()` and `ticks()` end-to-end.


- **WASM suite subprocess isolation; run-one.mjs helper** (W1.13) — Each test in
  `tests/wasm/suite.mjs` now runs in its own Node.js subprocess via `spawnSync` +
  `tests/wasm/run-one.mjs`.  Previously, a WASM crash (`RuntimeError: unreachable`
  or `memory access out of bounds`) in one test corrupted the shared module's linear
  memory, causing all subsequent tests in the same process to also fail.  `run-one.mjs`
  loads a fresh `pkg/loft.js` module and VirtFS default tree per invocation and writes
  the JSON result to stdout.  `suite.mjs` no longer imports `createHost` /
  `buildDefaultTree` / `withFiles`; the subprocess helper owns that setup.

- **`wasm_compile_and_run_smoke` converted to real integration test** (W1.9) — The
  hollow `#[ignore]` placeholder in `tests/wasm_entry.rs` has been replaced by an
  integration test that runs `node tests/wasm/bridge.test.mjs` as a subprocess.
  The test skips gracefully when the WASM package is not built or Node.js is absent,
  and fails with a clear message when the bridge tests report a non-zero exit code.

- **`13-file.loft` removed from `WASM_SKIP`** — File I/O operations (`OpDelete`,
  `OpMoveFile`, `OpMkdir`, `OpMkdirAll`) now route through `codegen_runtime::fs_*`
  functions that compile cleanly for the `wasm32-wasip2` target.  The wasm32-wasip2
  compilation test (`wasm_dir`) no longer skips `tests/docs/13-file.loft`; `#74`
  is fully resolved.



- **WASM file I/O wired to VirtFS host bridge** (W1.16) — All file operations
  (`read_text`, `write_text`, `read_bytes`, `write_bytes`, `seek`, `file_size`,
  `truncate`, `is_file`, `is_dir`, `list_dir`, `delete`, `move`, `mkdir`,
  `mkdir_all`) now call `globalThis.loftHost.*` via `js_sys::Reflect` under the
  `wasm` feature.  Helpers `assemble_write_data` and `dispatch_read_data` extracted
  from `state/io.rs` to share assembly logic between WASM and native paths and
  satisfy clippy `too_many_lines`.  `tests/wasm/bridge.test.mjs` gains three binary
  I/O tests (BigEndian write/read, seek + partial read, truncate); `doc/claude/ROADMAP.md`
  updated to mark W1.16 as done.

- **WASM skip for lock functions removed** (W1.17) — `n_get_store_lock` and
  `n_set_store_lock` are resolved from `loft::codegen_runtime` (listed in
  `CODEGEN_RUNTIME_FNS` in `generation/mod.rs`), so no `todo!()` stub is emitted.
  `18-locks.loft` removed from `WASM_SKIP`; the WASM compilation test now exercises
  `#lock` attribute syntax and `get_store_lock()` / `set_store_lock()`.

- **WASM skip for function references removed** (W1.15) — `output_call_ref` in
  `emit.rs` generates a `match` dispatch over all reachable definitions with a
  matching signature, implementing fn-ref calls (`f(args)` where `f: fn(T) -> R`)
  in native/WASM output.  `06-function.loft` removed from `WASM_SKIP`; the WASM
  compilation test now exercises function references, lambdas, and higher-order
  functions (`map`, `filter`, `reduce`).

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

- **Native coroutine `yield from` delegation** (N8b.3) — `yield from sub_gen()`
  now works in native-compiled generators.  The sub-generator is stored as
  `Option<Box<dyn LoftCoroutine>>` directly in the outer struct, avoiding the
  `NATIVE_COROUTINES` `RefCell` that would cause a "RefCell already borrowed" panic
  when the outer `next_i64` tries to advance the inner generator.  The outer
  `next_i64` body is wrapped in a `loop {}` when yield-from segments are present;
  exhausted sub-generators set the next state and `continue` immediately.  Factory
  functions for sub-generators are called directly (not via `alloc_coroutine`) so
  sub-generators are never registered in the shared table.  CO1.4 test in
  `51-coroutines.loft` (`outer_with_from` producing 1+10+20+2 = 33) now passes.

- **Native coroutine state-machine code generation** (N8b.1, N8b.2) — Generator
  functions (`fn foo() -> iterator<integer>`) are now supported by the `--native`
  Rust backend.  Each generator is translated into a hand-written Rust state-machine
  struct (e.g. `NCountGen { state: u32, … }`) implementing the new `LoftCoroutine`
  trait (`fn next_i64(&mut self, stores: &mut Stores) -> i64`).  The coroutine body
  is split at `yield` nodes into match arms; a catch-all `_ =>` arm returns
  `COROUTINE_EXHAUSTED` (= `i32::MIN as i64`).  Three new pieces land in
  `src/codegen_runtime.rs`: the `LoftCoroutine` trait, a thread-local
  `NATIVE_COROUTINES` table (avoiding changes to `Stores`), `alloc_coroutine`,
  `coroutine_next_i64`, and `coroutine_is_exhausted`.  Call sites emit
  `loft::codegen_runtime::alloc_coroutine(foo(stores, args))` via a new
  `src/generation/coroutine.rs` module.  `OpCoroutineNext` and `OpCoroutineExhausted`
  are dispatched in `src/generation/dispatch.rs`.  `collect_calls` in
  `src/generation/mod.rs` now walks `Value::Yield` nodes so helper functions called
  from yield expressions are included in the reachable set.  `51-coroutines.loft`
  removed from `SCRIPTS_NATIVE_SKIP`; `native_scripts` passes all 4 generator tests.

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

### Test coverage

- **`single` (f32) type fully covered** — New `tests/scripts/52-single.loft` covers
  all previously zero-coverage `single` operations: arithmetic (sub, mul, div, rem),
  all six comparison operators, NaN null semantics and propagation, null coalescing,
  positive/negative infinity (non-null), conversions (`as single` from integer/float/text;
  `single as` float/integer/long/text), format specifiers, and NaN-producing casts.
  The test is registered in `tests/wrap.rs` as `single_type`.

### Closure improvements

- **Spurious closure diagnostics suppressed** (A5.6d) — The "closure record '…' created"
  diagnostic is now `Level::Debug` (invisible in normal output and tests).  Captured outer
  variables are now marked as read at the call site via `var_usages`, eliminating false-positive
  "Variable X is never read" and "Dead assignment" warnings for validly captured variables.
  Tests `closure_capture_integer`, `closure_capture_after_change`, `closure_capture_multiple`,
  `closure_capture_text_integer_return`, and `closure_capture_text_return` no longer assert
  spurious warnings.

- **Closure capture coverage tests added** (A5.6e) — Four new tests in `tests/expressions.rs`
  verify closures across data-source scenarios: `closure_capture_struct_ref` (12-byte DbRef
  capture), `closure_capture_vector_elem` (vector element capture), and the existing
  `closure_capture_text_return` / `closure_capture_text_integer_return` tests cover text captures.

- **Work buffer cleared before each closure call** (A5.6f) — The hidden work-buffer `String`
  is now cleared (`v_set(wv, "")`) before each `OpCreateStack` injection at call sites.  Without
  this fix, calling a text-returning lambda inside a loop accumulated text from previous iterations
  (e.g. `"hello, world!"` became `"hello, world!hello, world!"` on the second call).  New test
  `closure_capture_text_loop` in `tests/expressions.rs` verifies the fix.

- **`fn`-ref conditional assignment no longer SIGSEGVs** (A5.6h) —
  `f = if flag { inc } else { dec }` caused a SIGSEGV at the `CallRef` opcode.
  Root cause: a fn-ref slot is 16 bytes (`[d_nr 4B][closure DbRef 12B]`), but
  each branch of an if-else expression generated only 4 bytes (the d_nr via
  `OpConstInt`), because `generate_block` (called for each branch) was setting
  `stack.position = to + size(Function) = to + 16` without emitting any instruction
  to push the 12-byte sentinel.  This phantom advance caused the codegen stack
  tracker to skip `OpNullRefSentinel` and left `CallRef` reading from the wrong
  stack position (the frame header, containing d_nr=0, which dispatched to
  `i_parse_errors()` and then SIGSEGVed in `dump_stack` on a garbage text pointer).
  Fix: `generate_block` now emits `OpNullRefSentinel` when the block result type is
  `Type::Function` and the block's content pushed fewer than 16 bytes.  A defensive
  `gen_fn_ref_value` helper in `generate_set` handles non-Block fn-ref values.
  Additionally, three native-codegen regressions introduced in A5.6g were resolved:
  (1) `visible_attr_count` (not `def.attributes.len()`) is now used in the candidate
  filter for closure-capturing lambdas; (2) the closure work-variable is injected at
  call sites for closure-capturing dispatch; (3) `Value::FnRef(d_nr, …)` is added to
  `collect_int_fn_refs` and emits `{d_nr}_u32` in native output so closure lambda
  functions appear in the reachable set and are compiled.  Test: `fn_ref_conditional_call`
  in `tests/issues.rs`; all 8 closure interpreter tests and the full native suite pass.

- **Definition-time capture semantics and multi-call closure injection** (A5.6g) —
  Closures now capture variable values at definition time (when the lambda is written),
  not at call time (when it is first invoked).  `emit_lambda_code` allocates and
  populates the closure record inside the `fn_ref_with_closure` block — the block is
  the `*code` assigned to the fn-ref variable, so it runs exactly once at definition
  time.  A `closure_vars` fallback was restored in `src/parser/control.rs` (both
  `try_fn_ref_call` and `parse_call` paths): when `last_closure_alloc` has already
  been consumed by a first call site, subsequent call sites to the same fn-ref variable
  look up the closure work variable via `self.closure_vars.get(&v_nr)` and inject it
  as the hidden `__closure` arg.  This fixes `closure_capture_struct_ref` and
  `closure_capture_vector_elem`, which each call the lambda twice (condition + format
  string).  Native codegen was also fixed: `OpVarFnRef`/`OpStoreClosure` declarations
  were removed from `default/02_images.loft` (they would have overflowed the 254-entry
  OPERATORS array); the `output_call_ref` dispatch in `src/generation/emit.rs` now
  compares total attribute count (including `__closure`) against total args (since
  the closure is injected explicitly at the call site, not by `fn_call_ref`); the
  `OpGetClosure` injection was removed.  The block result type was changed to a
  full-range integer to prevent native codegen from emitting a truncating `as u8`
  cast that corrupted the d_nr dispatch value.  All 8 closure tests pass (1 ignored
  for cross-scope closures, a known limitation in CAVEATS.md C1);
  `tests/docs/26-closures.loft` updated to reflect definition-time semantics.

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

- **Mutable closure captures write back to outer scope after each call** (A5.6c)
  — Void-return lambda calls now emit a write-back sequence after the `CallRef`
  instruction: for each field of the closure record, `OpGetInt` (or the
  field-type equivalent) reads the updated value back and stores it to the
  corresponding outer-scope variable.  Two root-cause bugs were fixed along the
  way: (1) `closure_vars.insert` was executing before the RHS lambda was parsed
  (because the insert check ran before `parse_assign_op`, which is where the
  lambda tokens are consumed); (2) the write-back used `Value::Block` (which
  creates a new scope), causing `scopes.rs` to emit `OpFreeRef` for the closure
  variable at the inner scope exit — leaving a dangling DbRef for the second
  call.  The fix uses `Value::Insert` instead, keeping the closure record alive
  across all calls in the outer scope.
  Test `p1_1_lambda_void_body` in `tests/issues.rs` passes without `#[ignore]`.

- **Text capture via `CallRef` no longer produces garbage DbRef** (A5.6b.1) —
  In `generate_call_ref`, the `__closure` argument (a `DbRef`) was being pushed
  onto the wrong stack frame: it was placed at the stack position of `x`
  (the first explicit argument), not at the position expected by the lambda
  body.  Two separate code paths were fixed: (1) for zero-param fn-refs the
  fast path now injects the closure arg; (2) `text_return` no longer adds
  captured RefVar(Text) variables as spurious extra args to the lambda's
  parameter type, which previously caused arity-mismatch failures.

- **`generate_call_ref` pre-allocates text work buffers for closures** (A5.6b.2)
  — A spurious `debug_assert!(work_vars.is_empty())` in `generate_call_ref`
  fired when a capturing lambda returned text, because the closure record
  contains a RefVar work buffer.  The assert has been removed; the existing
  logic already handles non-empty `work_vars` correctly.  Test
  `closure_capture_text_integer_return` passes without `#[ignore]`.

- **`yield` inside `par(...)` body now produces a compile-time error** (P2-R6
  M11-a) — The parser sets an `in_par_body` flag while parsing the body block
  of a `for … par(…)` loop.  When `yield` is encountered with `in_par_body`
  true, an Error diagnostic is emitted: "yield is not allowed inside a
  par(...) parallel body".  The yield expression is still consumed (to keep the
  lexer in sync) but no coroutine IR is generated, so scope analysis does not
  see orphaned reference variables.  The `in_par_body` flag is saved and
  restored for nested par() bodies.  Test
  `p2_r6_yield_inside_par_body_rejected` in `tests/issues.rs` passes without
  `#[ignore]`.  The existing runtime out-of-bounds guard (S23 / M11-b) in
  `coroutine_next` remains as defence-in-depth.

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
