# Changelog

All notable changes to the loft language and interpreter are documented here.

This project follows [Semantic Versioning](https://semver.org/).
The stability guarantee is described in `doc/claude/RELEASE.md`.

---

## [Unreleased]

### New features

- **N1** — `--native` and `--native-emit` CLI flags added.  `--native <file.loft>`
  parses and compiles the program, emits a self-contained Rust source file to a
  temporary path, invokes `rustc --edition=2024` to produce a native binary, and
  runs it.  `--native-emit <out.rs>` emits the Rust source to a named file without
  compiling — useful for inspecting codegen output.  A clear error is printed if
  `rustc` is not in `PATH`.  (`src/main.rs`, `src/generation.rs`)

### Prototype features

- **P1.1** — Lambda expressions are now parsed as primary expressions.  An inline
  `fn(params) -> type { body }` at any expression position produces a
  `Type::Function` value — identical to a named fn-ref — without requiring a
  top-level function declaration.  Lambda bodies cannot capture outer variables
  yet (closure capture is A5).  (`src/parser/expressions.rs` `parse_lambda`)

- **P1.2** — Short-form lambda syntax `|x| { body }` and `|| { body }` is now
  accepted everywhere a `fn(…) -> …` type is expected.  Parameter types are
  inferred from the call-site hint when no explicit `: type` annotation is given.
  Return type is inferred from the hint when no `-> type` annotation is given.
  (`src/parser/expressions.rs` `parse_lambda_short`, `src/parser/mod.rs`
  `lambda_hint`, `src/parser/control.rs` hint propagation in `parse_call`)

- **P1.3** — `map`, `filter`, and `reduce` now accept short-form lambdas as
  their function argument.  Any call site where the expected parameter type is
  `fn(…) -> …` now propagates the type hint so that untyped lambda params can
  be inferred.  (`src/parser/control.rs`)

### Improvements

- **A8** — Destination-passing calling convention for the three text-returning native
  functions (`replace`, `to_lowercase`, `to_uppercase`).  The compiler now detects
  `r = text_fn(…)` and `r += text_fn(…)` patterns and emits `OpCreateStack` +
  `OpStaticCall` to a `_dest` variant of the native, which writes the result string
  directly into the destination variable via `push_str`.  The scratch buffer
  (`Stores.scratch`) is retained for all other call paths (e.g. format string
  interpolation `"{expr}"`) and cleared at statement boundaries by `OpClearScratch`.
  (`src/native.rs`, `src/state/codegen.rs`, `src/fill.rs`, `default/02_images.loft`)

- **S4** — `read_data` in `database/io.rs` now implements `Parts::Array` (indirect
  record reads by looping over element count and recursing per element).
  `Parts::Sorted | Ordered | Hash | Index | Spacial` now emits a clear panic
  message explaining that binary I/O is not supported for keyed collections, instead
  of the previous generic `"Not implemented"` panic.  `Parts::Base` is now
  `unreachable!`.  The same messaging improvements are applied to `write_data`;
  `Parts::Array` write support is deferred (requires allocating new records).
  (`src/database/io.rs`)

### Native codegen fixes

- **N3** — `output_set` now emits `OpCopyRecord` after a reference-to-reference
  assignment so generated code performs a deep copy instead of an aliasing pointer copy.
  (`src/generation.rs` `output_set`)

- **N7** — `output_call` now intercepts `OpFormatFloat`, `OpFormatStackFloat`, and
  `OpFormatStackLong` and emits direct calls to `ops::format_float` /
  `ops::format_long` with the correct `(&mut) String` argument instead of falling
  through to the template handler which produced broken `OpFormatFloat(stores, …)` code.
  (`src/generation.rs` `output_call` + new `format_float` helper)

- **N5** — `output_call` now emits `if var.rec != 0 { vector::clear_vector(…) }` for
  `OpClearVector` instead of an unconditional call.  `stores.null()` returns a DbRef
  with a real `store_nr` but `rec == 0`; the bare call caused a panic
  ("Unknown record 2147483648") when a vector-returning function initialised its result
  from null.  (`src/generation.rs` `output_call`)

- **N4** — `output_enum` now registers struct-enum variants with their actual struct
  `known_type` instead of `u16::MAX`.  `ShowDb` uses this type to dispatch to variant
  fields; the `u16::MAX` sentinel caused it to only output the variant name, omitting
  all struct fields.  (`src/generation.rs` `output_enum`)

- **N8** — `codegen_runtime.rs` now provides `OpSortVector`, `OpInsertVector`, and
  `OpLengthCharacter`.  Generated sort/insert/character-length operations had no
  link target, causing ~12 native test files to fail to compile.
  (`src/codegen_runtime.rs`)

- **N10** — `output_call_template` in `generation.rs` now wraps character-typed
  variables with `ops::to_char(…)` when the call template expects `char`, and appends
  `as u32 as i32` when a function returning `char` is assigned to an `i32` variable.
  Fixes char/i32 type mismatches in generated code for character method calls.
  (`src/generation.rs` `output_call_template`)

- **N2** — `output_init` now emits type definitions in dependency order (topological
  sort) so content types are always registered before the container structs that
  reference them.  Also fixes a cycle-detection gap in `finish_type`: mutually
  recursive type graphs (e.g. enum → struct → sorted<T> → T → enum) no longer cause
  infinite recursion in `Stores::finish()`.
  (`src/generation.rs` `output_init` + `emit_def_ordered`;
   `src/database/types.rs` `finish_type`)

- **N9a** — `create.rs::generate_code()` now emits `use crate::ops;` in the generated
  `tests/generated/fill.rs` header so the file can be compiled as a crate module
  without unresolved `ops::` references.  (`src/create.rs`)

- **N9 (N20b/N20d)** — `create.rs::generate_code()` now calls `rustfmt` on the generated
  `tests/generated/fill.rs` (N20b) so the file is formatted identically to
  `src/fill.rs`.  Six operators that previously generated `s.{op}()` delegation
  stubs now have `#rust` templates and generate real implementations (N20d):
  `OpMathFuncSingle` / `OpMathFunc2Single` (f32 match dispatch),
  `OpMathFuncFloat` / `OpMathFunc2Float` (f64 match dispatch),
  `OpClearScratch` (`stores.scratch.clear()`), and `OpSortVector`
  (inlined elem\_size + is\_float + `vector::sort_vector` call; the inline
  also replaces the `OpSortVector` call that previously required a
  `codegen_runtime` entry).  `src/fill.rs` is replaced by the auto-generated
  version; a new `n9_generated_fill_matches_src` test enforces the invariant.
  (`src/create.rs`, `default/01_code.loft`, `default/02_images.loft`,
   `src/fill.rs`, `tests/issues.rs`)

### Bug fixes

- **L4** — Passing an empty vector literal `[]` directly as a mutable `vector<T>`
  function argument no longer panics in debug builds.  `parse_vector` now creates a
  named temporary in the else branch (call-site `[]` with no surrounding variable or
  field context) and emits the `vector_db` initialisation ops for that temporary on the
  second pass, giving `generate_call` a real 12-byte `DbRef` to pass as the mutable
  argument.  (`src/parser/expressions.rs` `parse_vector`)

- **L5** — `v += extra` through a `&vector<T>` ref-param now correctly appends to the
  caller's vector.  Two root causes were fixed: (A) the `RefVar(Vector)` path in
  `parse_append_vector` was emitting a raw stack-offset displacement instead of a proper
  dereferenced `DbRef`, so `vector_append` read from the wrong address; (B) there was no
  write-back of the updated `DbRef` after a potential reallocation.  The fix creates a
  local temporary, emits an explicit deref via `OpGetStackRef`, calls `OpAppendVector` on
  the temporary, and writes back via `OpSetStackRef`.  A second fix in `codegen.rs`
  corrects a slot-collision when a `&vector<T>` ref-param precedes a block argument: the
  block's scratch space was overwriting the ref-param's stack slot.
  (`src/parser/expressions.rs` `parse_append_vector`;
   `src/state/codegen.rs` generate-call ref-param path)

### Improvements

- **Two-zone slot assignment** — `assign_slots` redesigned to eliminate TOS-estimate errors.
  Large variables (Text 24 B, Reference 12 B, Vector 12 B) are placed sequentially in
  IR-walk order (`place_large_and_recurse`); small variables (≤ 8 B) are coloured within a
  pre-claimed zone at the frame base.  A new `Block.var_size` field stores the zone-1 size;
  `generate_block` emits `OpReserveFrame(var_size)` at block entry to claim the small-variable
  frame upfront.  `validate_slots` (debug-only) checks for slot conflicts and reports
  overlapping live ranges.  The old `assign_slots_old`, `eager_slots`, and `running_tos`
  machinery is removed.  Design notes: `doc/claude/SLOTS.md`.
  (`src/variables.rs`, `src/state/codegen.rs`, `src/data.rs`, `default/01_code.loft`,
  `src/fill.rs`, `src/state/mod.rs`)

- **A13** — Float (`f64`, 8 B) and Long (`i64`, 8 B) variables now reuse dead stack
  slots of the same size, matching the existing behaviour for `integer` and `single`.
  Neither type has a pre-init opcode so slot reuse is safe.  (`src/variables.rs`
  `assign_slots`: `can_reuse` threshold raised from ≤ 4 to ≤ 8 bytes)

- **A14** — `clean_work_refs` no longer mutates a work-ref variable's `type_def` to
  `Reference(0, [0])` to suppress `OpFreeRef` emission.  A new `skip_free: bool` field
  on `Variable` carries this intent explicitly; `get_free_vars` in `scopes.rs` checks
  `is_skip_free()` before emitting `OpFreeRef`.  This keeps `type_def` intact for
  downstream passes that inspect the real type.  (`src/variables.rs`, `src/scopes.rs`)

- **A15** — `inline_ref_set_in` (parser) is now an exhaustive match over all `Value`
  variants instead of using a `_ => false` catch-all.  Adding a new compound `Value`
  variant without updating this function is now a compile error rather than a silent
  incorrect null-init placement.  (`src/parser/expressions.rs`)

- **S3** — `find()`, `next()`, and `remove()` in `search.rs`, and the inner
  `Parts` dispatch in `read_data()`/`write_data()` in `io.rs`, now use exhaustive
  match arms listing every known `Parts` variant explicitly instead of a `_ =>` catch-all.
  Adding a new `Parts` variant without updating these dispatch sites is now a compile
  error rather than a silent fall-through to a misleading panic.
  (`src/database/search.rs`, `src/database/io.rs`)

- **S4** — `read_data` and `write_data` in `database/io.rs` now correctly handle
  `Parts::Struct` fields: each field is accessed at `r.pos + f.position` instead of
  the same `r.pos` for every field (previously every field in a struct would read the
  first field's value).  A new `binary_size()` helper computes the byte width of each
  type so that `write_data` advances the data-slice offset between fields.  Collection
  variants (`Array`, `Sorted`, `Ordered`, `Hash`, `Index`, `Spacial`) are now
  `unreachable!()` with a message referencing the F57 compile-time guard that blocks
  those field types at parse time.  `Parts::Base` is likewise `unreachable!()`.  Unit
  tests in `database::io::tests` verify that both read and write correctly distinguish
  field `a` from field `b` in a two-field struct.  (Issues 59, 63;
  `src/database/io.rs` `read_data`/`write_data`/`binary_size`)

- **S4 (format)** — `Stores::path()` in `database/format.rs` no longer silently skips
  the `// TODO` branch for sub-struct traversal.  When a parent struct's field is itself
  a struct (or enum-value) containing a collection of the child type, `path()` now
  builds the path component as `"field.subfield[index]"`.
  (`src/database/format.rs` `Stores::path`)

- **S5** — `copy_claims_index_body` (database) replaces `unreachable!()` with an
  explicit `panic!` that names the type index and the unexpected `Parts` variant,
  giving a diagnostic instead of an opaque crash if called incorrectly.
  (`src/database/allocation.rs`)

- **S6-65** — `Stores::get_type(nr)` helper added to `database/mod.rs`; it panics with
  `"type index N out of range (total: M)"` instead of a generic bounds-check message,
  making schema-corruption diagnostics actionable.  (`src/database/mod.rs`)

- **S6-67** — `resize_store` now uses `saturating_mul(7)` for the growth-factor
  calculation, preventing u32 overflow when the store is very large.  `claim_grow`
  uses `checked_add` for the new-size arithmetic and panics with
  `"store size limit exceeded"` instead of wrapping silently to a smaller value that
  caused `resize_store` to return early, leaving the store under-allocated.
  (`src/store.rs`)

- **S6-64** — `resize_store` now asserts `to_size <= MAX_STORE_WORDS` before
  allocating, where `MAX_STORE_WORDS = i32::MAX as u32`.  Store offsets are stored
  as `i32` values; without this guard a store grown beyond 2 GiB words would silently
  produce negative offsets, corrupting all subsequent allocations.  (`src/store.rs`)

- **S6-66** — `debug_assert!` guards added before narrowing `as i32` casts in
  `gen_text` (long-text code offset and text length) and in `copy_vector` (vector
  allocation offset).  Overflow is caught in debug builds with a clear message before
  the silent truncation can corrupt the bytecode or the store.
  (`src/state/codegen.rs`, `src/database/allocation.rs`)

- **S1** — The parser now emits `"Unknown variable 'x'"` directly on the second
  pass when a name is not found, instead of creating a ghost `Type::Unknown(0)`
  variable that propagated as a confusing downstream type error.  A typo in a
  variable name now produces a single, accurate diagnostic at the point of use.
  (`src/parser/expressions.rs`)

- **S2** — Recursion depth limits (1000 levels) added to `compute_intervals`,
  `inline_ref_set_in`, `generate`, and `scan` to prevent stack overflows on
  pathologically nested expression trees.  `compute_intervals` and
  `inline_ref_set_in` use an explicit `depth: usize` parameter; `generate` and
  `scan` use a struct field (`generate_depth`, `scan_depth`) to avoid cascading
  signature changes.  (`src/variables.rs`, `src/parser/expressions.rs`,
  `src/state/codegen.rs`, `src/scopes.rs`)

- **A6.4** — `claim()` and `assign_slots_safe` removed; `LOFT_DEBUG_SLOTS` debug blocks
  deleted from both `variables.rs` and `codegen.rs`.  `claim()` is replaced by
  `set_stack_pos()` — a minimal method that only sets the slot position — with the
  caller advancing `stack.position` separately.  The TOS-drop fallback in `generate_set`
  (pre-assigned slot above current TOS after an if-else) calls `set_stack_pos(v,
  stack.position)` to override the slot to TOS so direct placement fires correctly.
  (`src/variables.rs`, `src/state/codegen.rs`)

- **A6.3b** — Greedy interval-colouring slot assignment (`assign_slots`) is now the
  unconditional default.  Three bugs fixed: (B) narrow→wide dead-slot reuse rejected
  by requiring exact size match in the reuse guard; (C) `Value::Iter` now recurses
  into `create`/`next`/`extra_init` sub-expressions so index variables get correct
  `last_use` values; (C part 2) `Value::Set` now updates `last_use` for the write
  target so write-only variables are not treated as dead.  The `LOFT_ASSIGN_SLOTS`
  and `LOFT_LEGACY_SLOTS` env-var gates are removed.  (`src/variables.rs`,
  `src/scopes.rs`)

- **A6.3a** — Safe slot pre-pass (`assign_slots_safe`) is now the default codegen path.
  Variables receive sequential slots in `first_def` order with no reuse; `claim()` in
  codegen is retained but skipped for already-allocated variables.  An `is_stack_allocated`
  flag on each variable replaces the fragile `pos == u16::MAX` sentinel.  The greedy
  coloring path is retained behind `LOFT_ASSIGN_SLOTS=1`; the legacy pure-claim path
  behind `LOFT_LEGACY_SLOTS=1`.  `compute_intervals` now correctly handles
  `needs_early_first_def` (Text/Reference/nullable Enum only — Float and Long excluded)
  and extends loop-carried variable lifetimes.  The shadow-comparison pass from A6.2 is
  removed.  (`src/variables.rs`, `src/scopes.rs`, `src/state/codegen.rs`,
  `doc/claude/SLOT_FAILURES.md`)

- **A3** — `png`, `mmap-storage`, `rand_core`, and `rand_pcg` are now optional
  Cargo features (`png`, `mmap`, `random`).  The default feature set keeps all
  three enabled so existing builds are unaffected.  Building with
  `--no-default-features` produces a minimal binary with no image loading,
  file-mapped stores, or RNG.  `Store::open` panics at runtime when invoked
  without the `mmap` feature; `Database::get_png` returns `false`; the `rand`,
  `rand_seed`, and `rand_indices` functions are not registered.  CI matrix now
  includes a `cargo build --no-default-features` step to prevent regressions.
  (`Cargo.toml`, `src/lib.rs`, `src/main.rs`, `src/store.rs`, `src/ops.rs`,
  `src/native.rs`, `src/database/io.rs`, `.github/workflows/ci.yml`)

- **N6.3** — Reverse and range-bounded iteration on sorted/index collections is
  fully implemented in native codegen.  `codegen_runtime.rs` handles the reverse
  bit (64) in `OpIterate` and non-zero `from`/`till` key counts for range bounds;
  no code changes were required — this was discovered to already be complete.
  Tests `n6_sorted_reverse_native` and `n6_index_range_iteration` confirm both
  paths.  (`tests/vectors.rs`)

- **A11** — Hash table load-factor threshold corrected from ~57% to 75%
  (`src/hash.rs`: `length * 14 / 16` → `length * 2 / 3`). The previous formula
  rehashed prematurely due to the `elms = (room-1) * 2` geometry. DEVELOPERS.md
  updated accordingly.

### Bug fixes

- **Issue 72** — `map`, `filter`, and `reduce` comprehensions no longer produce slot
  conflicts.  In `place_large_and_recurse`, when a large non-Text variable is initialised
  from a block (`Set(outer, Block([Set(inner, ...), Var(inner)]))`), the inner block is now
  processed with `frame_base = outer.stack_pos` instead of `outer.stack_pos + outer.size`.
  This matches codegen: `generate_block` runs with `to = outer.stack_pos`, so both
  `outer` and `inner` share the block's result slot legally (non-overlapping live intervals
  in parent/child scopes).  A `debug_assert!(pos <= stack.position)` guard added to
  `generate_set` catches any future regression; an unconditional `assert!(pos != u16::MAX)`
  added to detect variables that escape `assign_slots` before they corrupt the stack.
  (`src/variables.rs` `place_large_and_recurse`; `src/state/codegen.rs` `generate_set`)

- **F57** — `write_file` and `read_file` on a struct that contains a `sorted<T>`,
  `index<T>`, or `hash<T>` field now emits a compile-time error ("cannot use
  write_file/read_file on a struct with collection-type fields") instead of panicking at
  runtime.  (`src/native.rs` `has_collection_field` check)

- **A9** — Assigning a vector slice to a variable (`s = v[a..b]`) now materialises the
  slice into an independent copy.  Mutating `s` (e.g. `s += [x]`) no longer corrupts the
  original vector `v`.  Appending a slice (`v += v[a..b]`) also produces correct results.
  (`src/parser/expressions.rs` — A9 handler in `parse_assign_op`,
  `build_vector_list` first-pass guard with `!first_pass && !is_argument && !u16::MAX`)

- **T0-8** — Seven `panic!`/`unreachable!` calls in the parser converted to `diagnostic!`
  + early return. Malformed input now produces an error message instead of crashing the
  compiler.

- **T1-31** — All integer and long arithmetic operators in `ops.rs` now use checked
  arithmetic in debug builds. Overflow panics with a clear message; results that collide
  with the null sentinel (`i32::MIN` / `i64::MIN`) are also caught. Release builds retain
  the fast unchecked path.

- **P20** — `f#next = pos` (file seek) before the first read or write now stores the
  position in `#next` so the first I/O operation applies the pending seek.

- **P45** — `&vector` parameter no longer triggers a false "never modified" warning
  when the function body uses `OpClearVector`, `OpInsertVector`, or `OpRemoveVector`.

### Diagnostics

- **T1-26** — Match exhaustiveness error now points at the `match` keyword instead of the
  closing brace. Unused-definition warnings now use `at file:line:col` format instead of
  Rust Debug formatting.

- **T0-9** — `read_to_string().unwrap()` in `get_file_text` now gracefully clears the
  buffer on non-UTF-8 file data instead of panicking the runtime. (PROBLEMS #48)

- **T0-10** — Invalid UTF-8 in a `.loft` source file now emits a Fatal diagnostic
  ("Cannot read line N — is the file valid UTF-8?") instead of silently truncating
  parsing at the bad line. Both `next()` code paths in `lexer.rs` are covered. (PROBLEMS #47)

### Diagnostics

- **T1-29** — Downgraded three `Level::Fatal` diagnostics to `Level::Error`:
  "use statements must appear before all definitions", "Syntax error" (now includes
  the unexpected token), and "Cannot redefine" in `data.rs`. Parsing now continues
  after these errors and can report multiple issues.

- **T1-27** — Appended fix suggestions to six common error messages:
  - "Variable cannot change type" → "; use a new variable name or cast with 'as'"
  - "Cannot modify const" → "; remove 'const' or use a local copy"
  - "match not exhaustive" → "; add the missing variants or a '_ =>' wildcard"
  - "Cannot iterate" → "; expected vector, sorted, index, text, or range"

- **T1-30** — Expanded match documentation in LOFT.md to explain why guarded arms
  do not count toward exhaustiveness, with a code example.

---

## [0.8.0] — 2026-03-17

### Bug fixes

- **Shift by 0** — `x << 0` and `x >> 0` now correctly return `x` instead of null.
  The `v2 != 0` guard in `ops.rs` was meant to catch null but also caught legitimate
  zero shifts. Removed for all four shift functions (int/long × left/right). (2026-03-17)

- **Float null comparisons** — `NaN != x` now returns `true` (was `false`). All float
  and single comparison operators (==, !=, <, <=) now explicitly check `is_nan()`.
  Also fixed `??` operator for floats: changed from `x != null` comparison to boolean
  truthiness check. (2026-03-17)

- **If-expression without else** — Using `if` as a value expression without an `else`
  clause is now a compile error instead of silently producing null. If-statements
  (void body) are unaffected. (2026-03-17)

- **T0-1** — `null` literal in scalar field assignment emitted no bytecode, causing
  `OpSetInt` to misread the stack (`store_nr=60` crash). Fixed in `parse_assign_op`:
  `convert()` now resolves `Type::Null` to the typed-null constant before `towards_set`.
  Five regression tests in `tests/issues.rs`. (2026-03-15)

- **T0-2** — `OpFreeRef` was emitted in forward variable-declaration order; `database::free()`
  enforces LIFO. Functions with 2+ owned references panicked "Stores must be freed in LIFO
  order". Fixed by adding `var_order: Vec<u16>` to `Scopes`; `variables()` now iterates in
  reverse. (2026-03-15)

- **T0-3** — T0-1 fix regression: `convert()` ran unconditionally for all null assignments,
  rewriting `Value::Null` before `towards_set_hash_remove` could intercept it as a
  collection-remove. Fixed by guarding `convert()` to non-reference, non-collection types
  only. Restores `sorted[key] = null`, `hash[key] = null`, and `index[k1,k2] = null` removal.
  (2026-03-15)

- **T0-4** — `v += other_vec` (PROBLEMS #39): `vector_add()` appended element bytes via raw
  `copy_block` without calling `copy_claims()`. Text-field slot indices were shared between
  source and destination; end-of-scope free of one vector corrupted the other ("Unknown
  record N"). Fixed by adding a `copy_claims()` loop over each appended element after the
  block copy, mirroring `copy_claims_seq_vector()`. (2026-03-15)

- **T0-5** — `index<T>` as struct field (PROBLEMS #40): `copy_claims()` and `remove_claims()`
  in `allocation.rs` both reached `panic!("Not implemented")` for `Parts::Index`. Any
  `OpCopyRecord` or struct reassignment on a struct containing an `index<T>` field panicked.
  Fixed by adding `collect_index_nodes` (in-order RB-tree traversal) and
  `copy_claims_index_body` helpers, and inline Index arms in both match statements. Also
  added `#[cfg(debug_assertions)]` bounds checks to `Store::copy_block` and
  `Store::copy_block_between`. Three regression tests in `tests/issues.rs`. (2026-03-15)

- **T0-7** — `16-parser.loft` failed with a codegen assertion: `generate_call` reported a
  mutable Reference argument size mismatch (PROBLEMS #42). Root cause: `Code.define()` in
  `lib/code.loft` stored `res: i32` directly into `hash<Definition[name]>` via
  `self.def_names[name] = res` — a 4-byte integer where a 12-byte Reference was expected.
  Three further bugs uncovered when the compile error was fixed: (1) `get_type()` read
  `def_names[name].typedef` (always 0) instead of `definitions[nr].typedef`; (2) `structure()`
  in `lib/parser.loft` called `type_def()` which internally reset `cur_def` to null, making
  the following `argument()` call a no-op so struct fields were not registered; (3) `object()`
  had an inverted loop-break condition (`!test("}}") { break }`) causing struct literals
  with more than one field to abort after the first. Additionally the original `!= null`
  reference comparison generated `ConvRefFromNull()` (a store-allocating opcode) with no
  matching `FreeRef`, leaking one store per `define()` call and eventually causing a LIFO
  store-free panic. Fixes: store a full `Definition` in `def_names`; look up typedef through
  `definitions[nr]`; use integer null-check (`nr != null`) to avoid store allocation; restore
  `self.code.cur_def` in `structure()` after `type_def()`; correct `object()` loop condition.
  `16-parser.loft` removed from `SUITE_SKIP`; `wrap::last` re-enabled. (2026-03-16)

- **T0-6** — Inline ref-returning method calls leaked database stores → LIFO panic
  (PROBLEMS #41). `p.shifted(1.0, 0.0).x` synthesised an anonymous work-ref variable
  (`__ref_1`) via `parse_part()`, but its `OpNullRefSentinel` null-init was inserted after
  the first user statement — often BEFORE body variables like `p` in the block. `scan_set`
  then placed the work-ref before `p` in `var_order`, so reversed `var_order` freed `p`
  (store 2) before the work-ref (store 3+), violating LIFO. Fixed by inserting each
  work-ref's null-init immediately before the statement that first assigns it (found by
  recursively searching the block for `Set(r, _)` nodes). This guarantees the work-ref
  appears after `p` in `var_order` and is freed before `p` in the reversed order.
  Supporting changes: `OpNullRefSentinel` opcode; sentinel guards in `Stores::free/valid`;
  `Function::copy/append` preserve `inline_ref_vars`. `tests/docs/17-libraries.loft`
  removed from `SUITE_SKIP`. (2026-03-15)

- **T1-5 correctness** — `validate_slots` emitted false-positive "slot conflict" panics for
  same-name variables reused across sequential blocks. Fixed: `find_conflict` now exempts
  same-name/same-slot pairs; P1 pre-init added for ref-typed variables. (2026-03-13)

- **PROBLEMS #33/34/35** — Sorted filtered loop-remove gave wrong result; index key-null
  removal left 1 record; index loop-remove panicked "Unknown record". All fixed. (2026-03-14)

- **Various** — Null-coalescing `??`; non-zero exit on parse/runtime error; for-loop mutation
  guard extended to field access; reverse iteration on `sorted<T>`; CLI args in `fn main`;
  zero-pad format sign order; XOR-null bug for `^`/`|`/`&`; missing polymorphic method
  compiler panic; `for c in enum_vector` infinite loop. (2026-03-13/14)

### Features

- **T1-15** — Or-patterns in match arms. `North | South => "vertical"` and
  `1 | 2 | 3 => "low"` now work for both enum and scalar match expressions.
  Each variant in an or-pattern counts for exhaustiveness. EnumArm.disc
  refactored to Vec<i32> for multi-discriminant support. (2026-03-17)

- **T1-20** — Null and character patterns in match expressions. `null => "absent"`
  matches null values in scalar match arms. Character literals `'a' => "vowel"`
  are now recognized by the pattern parser. Wildcard binding (`@`) and
  name-binding patterns remain for a follow-up. (2026-03-17)

- **T1-23** — For-loop variable type mismatch error. Reusing a variable name in a
  for-loop with a different type (e.g. `x = 1.5; for x in int_vec`) now produces
  a compile error. Same-type reuse is idiomatic and not flagged. (2026-03-17)

- **T1-25** — `sizeof(u8)` now returns 1 (packed field size) instead of 4 (stack slot
  size). Range-constrained integer types (`u8`, `i8`, `u16`, `i16`) report their packed
  byte size, consistent with `sizeof(Struct)` for structs containing those fields.
  (2026-03-17)

- **T1-24** — Documented null sentinel values for every type in the language reference
  (LOFT.md § "Null representation"). Includes the `i32::MIN` arithmetic risk and
  mitigation via `long` or `not null`. (2026-03-17)

- **T1-16** — Guard clauses in match arms. Match arms now support `if guard`
  after the pattern: `Circle { r } if r > 0.0 => ...`. Guard failure falls
  through to the next arm. Guarded arms don't count for exhaustiveness.
  Works for enum, struct-enum, and scalar match expressions. Seven tests
  in `tests/match.rs`. (2026-03-17)

- **T3-9** — Scoped scratch reset. `OpClearScratch` opcode clears the
  temporary string buffer at every statement boundary. Native text functions
  (`replace`, `to_lowercase`, `to_uppercase`) no longer leak one `String`
  per call for the entire program run. (2026-03-17)

- **T1-17** — Range patterns in match expressions: `1..=9` (inclusive) and
  `10..100` (exclusive). Lowered to short-circuit AND condition.
  Three tests in `tests/match.rs`. (2026-03-17)

- **P46** — Match arms can now use block expressions `{ ... }` as bodies.
  The parser detects `{` after `=>` and parses it as a scoped block. Was a
  segfault because the block's `}` was confused with the match's `}`. (2026-03-17)

- **T1-24** — Commas between match arms are now mandatory (trailing comma before
  `}` is optional). Consistent with struct fields, enum variants, and function
  arguments. (2026-03-17)

- **T1-14** — Scalar patterns in match expressions. Match subjects can now be
  integer, long, float, single, text, boolean, or character values. Arm patterns
  are literals; lowers to an if/else chain. Unblocks T1-15, T1-16, T1-17.
  Three tests in `tests/match.rs`. (2026-03-17)

- **N15** — Generated if-expressions without an else branch now emit typed null
  sentinels (`i32::MIN`, `""`, `f64::NAN`, etc.) instead of `()`. Fixes ~4 compile
  failures and ~4 runtime failures. Native: 55 compile, 49 pass. (2026-03-17)

- **N18** — Fix `crate::state::` template references in generated native code.
  Substitutes with `loft::state::` so constants like `STRING_NULL` resolve
  correctly in standalone generated files. (2026-03-17)

- **T2-8** — Expose `vector.clear()` as a public stdlib function. Wraps the
  existing `OpClearVector` bytecode operator. Removes all elements from a vector,
  setting its length to 0. (2026-03-17)

- **T2-7** — `mkdir(path)` and `mkdir_all(path)` stdlib functions for creating
  directories. Both return true on success, false on failure. Paths validated
  against the project directory. (2026-03-17)

- **T1-18** — Plain struct destructuring in `match` expressions. Match subjects
  can now be plain struct types (not just enums). A struct pattern binds fields
  directly: `match p { Point { x, y } => x + y }`. (2026-03-17)

- **N10** — Add `OpCopyRecord` to `codegen_runtime` (deep struct copy with
  `copy_block` + `copy_claims`). 51 of 85 generated files compile, 45 pass.
  (2026-03-17)

- **N10** — Fix variable shadowing in generated native code. `output_function()` now
  populates `self.declared` with argument var numbers directly, preventing inner-block
  variable shadows that caused infinite recursion. 45 of 50 compiled files now pass
  (up from 44). (2026-03-17)

- **N10** — Fix type registration in generated native code. `output_native_reachable()`
  now registers ALL types (0..till) in `init()` so runtime type IDs match compile-time
  IDs. Native pass rate: 44 of 50 compiled files now pass (up from 24). (2026-03-17)

- **N7** — Native test suite. `native_test_suite` in `tests/expressions.rs` compiles
  and runs all generated test files when `LOFT_TEST_NATIVE=1` is set. Reports compile
  rate and execution pass rate. Baseline: 50 compile, 24 pass of 85 files. (2026-03-17)

- **N4+N5+N6** — Native codegen: handle `Value::Keys` in code generation (emit key
  array literal); skip all Op functions with no IR body; add `OpGetTextSub`,
  `OpSizeofRef`, `OpConvTextFromNull`, `OpConvRefFromNull` handlers; add compilation
  gate test `generated_code_compiles`. 46 of 86 generated files compile. (2026-03-17)

- **N3** — Native codegen runtime module (`src/codegen_runtime.rs`). Wrapper functions
  for database operations: `OpDatabase`, `OpNewRecord`, `OpFinishRecord`, `OpFreeRef`,
  `OpFormatDatabase`, `OpGetRecord`. Special-case handlers in `generation.rs` for
  `OpDatabase` (reassignment) and `OpFormatDatabase` (`&mut String`). 42 of 86 generated
  test files now compile. (2026-03-17)

- **N1+N2** — Native Rust code generation fixes. Fixed `#rust` templates:
  `external::` → `ops::`, `u32::from(@fld)` → `((@fld) as u32)`, added
  `s.database.` → `stores.` substitution in `generation.rs`. Added `n_assert`
  stub and `todo!()` stubs for native functions in generated test files.
  26 of 86 generated files now compile (from 0 before). (2026-03-17)

- **T1-22** — Missing return path warning for `not null` return types. Functions declared
  with `-> type not null` that may fall through without returning now warn "Not all code
  paths return a value — function 'name' may return null". Nullable return types (`-> type`)
  keep the existing error. Also fixes a false-positive "void should be X" error when all
  branches of an if/else use explicit `return`. Adds `definitely_returns` predicate and
  `returned_not_null` field to `Definition`. Five tests in `tests/parse_errors.rs`. (2026-03-16)

- **T1-12** — Redundant null check on `not null` field. Comparing a `not null` struct
  field to `null` with `==` or `!=` now warns ("comparison is always false/true").
  Using `??` (null-coalescing) on a `not null` field warns ("default is never used").
  The check is purely type-driven — `get_field` tracks the field's non-nullable status
  and `handle_operator` emits the warning before processing the operator. Five tests
  in `tests/expressions.rs`. (2026-03-16)

- **T1-13** — Unreachable code warning. Statements after an unconditional `return`,
  `break`, or `continue` at block scope now warn "Unreachable code after return".
  Only top-level terminators trigger the warning — a `return` inside an `if` branch
  does not mark the enclosing block as terminated. Four tests in
  `tests/parse_errors.rs`. (2026-03-16)

- **T1-10** — Unused loop variable warning. `for i in 0..10 { total += 1 }` now
  warns "Variable i is never read" when the loop variable is not referenced in
  the body. Prefix with `_` (e.g. `for _i in`) to suppress. Also fixed:
  `v#count`, `v#first`, `v#remove` etc. now correctly mark the base variable as
  used, preventing false positives. (2026-03-16)

- **T1-4** — `match` expression for enum dispatch with compiler-checked exhaustiveness.
  Plain enums dispatch on variant equality; struct-enum arms optionally bind fields
  (`Circle { radius } => ...`). All arms must return compatible types; missing variants
  without a wildcard `_ =>` are a compile-time error; duplicate variant arms produce a
  warning. Lowers to a `Value::If` chain — no new IR nodes or opcodes. Resolves
  INCONSISTENCIES #6 (plain enums can now have free-function dispatch via `match`).
  17 tests in `tests/match.rs`. (2026-03-16)

- **T1-2** — Wildcard and selective imports. `use mylib::*` imports all names from `mylib`
  into the current scope; `use mylib::Point, add` imports only the named items. Local
  definitions shadow imported names (local wins). Importing a name that does not exist
  produces a compile-time error. Three tests in `tests/imports.rs`. (2026-03-16)

- **T2-0** — Code formatter (`loft --format`). Token-stream formatter for `.loft` files.
  `loft --format file.loft` formats in-place; `loft --format -` reads stdin and writes
  stdout; `loft --format-check file.loft` exits 1 if the file is not in canonical format.
  Rules: 2-space indent, opening brace on same line as header, every block body multi-line,
  spaces around binary/assignment operators and `->`, fields on separate lines in struct/enum
  bodies, trailing commas stripped. Implemented in `src/formatter.rs` as a standalone
  tokenizer + state machine (no lexer changes). CRLF-safe on all platforms. 11 tests in
  `tests/format.rs`; cross-platform LF enforcement via `.gitattributes`. (2026-03-16)

- **T2-6** — `now()` and `ticks()` time functions. `now()` returns milliseconds since
  the Unix epoch (wall clock); `ticks()` returns microseconds elapsed since program start
  (monotonic). `Stores` gains a `start_time: Instant` field initialised at `new()` and
  cloned into parallel worker stores. Declared in `default/02_images.loft`; four tests
  in `tests/time.rs`. (2026-03-16)

- **T2-11** — External library package layout (`loft.toml`). `use mylib;` now
  resolves the packaged directory layout `<dir>/<id>/src/<id>.loft` in addition to
  the existing flat `<dir>/<id>.loft` layout. A minimal `loft.toml` manifest reader
  (`src/manifest.rs`) validates the `loft = ">=X.Y"` interpreter version requirement
  and reads the optional `[library] entry` override. Discovered via `lib_dirs` (steps 7c)
  and `LOFT_LIB` (step 7d) in `lib_path()`. Six tests in `src/manifest.rs` and
  `tests/package_layout.rs`. (2026-03-16)

- **T1-11** — Compile-time warning for division or modulo by constant zero. `n / 0` and
  `n % 0` return null in loft rather than panicking, so a constant-zero divisor is a
  completely silent bug. The parser now emits a warning when the right-hand operand of
  `/` or `%` is a literal `0` (integer or long). Two regression tests in
  `tests/expressions.rs`. (2026-03-16)

- **T1-1** — Callable fn-ref variables: `f(args)` where `f` holds a `fn` reference, and
  `fn`-typed function parameters. (2026-03-15)

- **T1-3** — `map`, `filter`, `reduce` in the standard library. Compiler special-cases in
  `parse_call`; cannot be expressed in plain loft (no generic type parameters). (2026-03-15)

- **T3-4 pre-gate** — `spacial<T>` now emits a compile-time error instead of panicking at
  runtime. (2026-03-15)

- **const unification** — compile-time local constants; `file#exists` separated from
  `file#format`; worker bytecode cloned once per `parallel_for` instead of per element;
  `store.claim()` O(n) scan replaced by LLRB tree. (2026-03-14)

### Infrastructure (post-0.1.0)

- Package renamed `dryopea` → `loft` in `Cargo.toml`, all source files, and generated tests.
- All game-engine branding strings removed from `src/data.rs`, `src/gendoc.rs`, HTML docs.
- README.md rewritten (language overview, install options, hello-world). CHANGELOG.md created.
- GitHub Actions CI (`ci.yml`: test on ubuntu/macOS/windows + clippy + fmt) and release
  pipeline (`release.yml`: 4-platform binaries, gh-pages HTML docs, crates.io publish).
- Clippy pedantic: all `#[allow(clippy::...)]` annotations justified or replaced; zero warnings.
- Source file splits: `parser.rs` (7687 lines) → `src/parser/` (6 modules);
  `database.rs` (3792) → `src/database/` (7 modules); `state.rs` (3525) → `src/state/` (5 modules).
- **R1** — Standalone `loft` GitHub repository created (`github.com/jjstwerff/loft`).
  Package renamed, game-engine content removed, README rewritten. (2026-03-16)

---

## [0.1.0] — 2026-03-15

First tagged release. All language features listed below are stable within the 0.1.x line.

### Language

- **Static type system** with inference — types are checked at compile time; mismatches are errors
- **Null safety** — every value is nullable unless declared `not null`; null propagates through arithmetic; `?? default` coalescing operator
- **Primitive types** — `boolean`, `integer`, `long`, `float`, `single`, `character`, `text`
- **Integer ranges** — `integer limit(0, 255)` (aliases `u8`, `u16`, `i8`, `i16`, `i32`)
- **Structs** — named field records with constructor syntax `T { field: value }`
- **Plain enums** — closed set of named values; comparison operators work across variants of the same enum
- **Struct-enums** — variants with different field sets; per-variant method dispatch (polymorphism)
- **Variables** — implicitly declared on first assignment; type inferred; `const` enforced at compile time
- **Control flow** — `if / else`, `while`, `loop`, `for in`, `break`, `continue`, `return`
- **For-loop filters** — `for x in v if condition { }` with `#first`, `#count`, `#index`, `#remove` attributes
- **Vector comprehensions** — `[for x in v { expr }]` and `[for x in v if cond { expr }]`
- **Ranges** — `1..10` (exclusive end), `rev(range)` for reverse iteration
- **String formatting** — `"{expr}"` interpolation; format specifiers `{x:06.2}`, `{x:>10}`, etc.
- **Operators** — arithmetic, comparison, logical, bitwise, `as` cast, `sizeof`, null-coalescing `??`
- **Functions** — top-level functions, methods (first `self` parameter), callable function references (`fn name`)
- **Const parameters** — `fn f(v: const T)` prevents mutation of the argument
- **`map` / `filter` / `reduce`** — stdlib higher-order functions accepting function references
- **`par(...)` for-loop clause** — `for a in items par(b=worker(a), threads) { ... }` distributes work across CPU cores using multiple threads
- **Type aliases** — `type Alias = ExistingType;`
- **Use declarations** — `use mylib;` loads a loft library; names are accessed as `mylib::Name`
- **Shebang** — `#!/usr/bin/env loft` supported on the first line

### Collections

- **`vector<T>`** — dynamic array; `+=` append, `[i]` index (null on out-of-bounds), slice `[a..b]`, `[elem; n]` repeat
- **`sorted<T[key]>`** — B-tree ordered by key field; `[key] = null` removes an element; forward and reverse iteration
- **`index<T[k1, k2]>`** — multi-key B-tree; compound key lookup; `[k1, k2] = null` removes
- **`hash<T[key]>`** — hash table; O(1) lookup by key field; `[key] = null` removes
- All collection types use `+=` to add elements and `for x in col` to iterate

### Standard library (default/)

- Math: `abs`, `sqrt`, `pow`, `floor`, `ceil`, `round`, `PI`
- Text: `len`, `trim`, `upper`, `lower`, `starts_with`, `ends_with`, `contains`, `replace`, `split`, `join`
- Collections: `len`, `clear`, `reverse`, `sort`, `map`, `filter`, `reduce`
- File I/O: open, read, write, seek, `file#exists`, `file#size`, directory listing
- Images: PNG read/write (`Image`, `Pixel`, `Format`)
- Logging: `log_info`, `log_warn`, `log_error`, `log_fatal` with source location and rate limiting
- Parallel: `par(...)` for-loop clause (compiler rewrites to internal `parallel_for`)
- Random: seeded PRNG (`random_int`, `random_float`)

### Compiler / interpreter

- Two-pass recursive-descent parser producing IR (`Value` enum)
- Bytecode compiler (`interpreter.rs`) and stack-based executor (`state.rs`)
- Slot assignment with liveness analysis (`compute_intervals`, `validate_slots`)
- Scope analysis emits `OpFreeText` / `OpFreeRef` at end-of-scope
- Diagnostic system — all parse and type errors emit a message with file:line:col; non-zero exit on error
- `--path` flag to override the project root (where `default/` is found)
- HTML documentation generator (`gendoc` binary)

### Known limitations

- **No lambda expressions** — anonymous functions planned for 1.1; `fn name` references work with `map`, `filter`, `reduce`, and the `par(...)` for-loop clause
- **No REPL** — interactive mode planned for 1.1
- **`sizeof(u8)` returns 4** — stack alignment means `sizeof` returns the stack slot size, not the byte-packed size; documented in `doc/claude/INCONSISTENCIES.md #23`

---

[Unreleased]: https://github.com/jjstwerff/loft/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/jjstwerff/loft/releases/tag/v0.1.0
