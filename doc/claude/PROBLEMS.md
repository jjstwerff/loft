# Known Problems in Loft

This document lists known bugs, unimplemented features, and limitations in the loft
language and its interpreter (`loft`). For each issue the workaround and the
recommended fix path are described.

## Contents
- [Open Issues — Quick Reference](#open-issues--quick-reference)
- [Runtime Crashes](#runtime-crashes)
- [Parser / Lexer Bugs](#parser--lexer-bugs)
- [Web Services Design Constraints](#web-services-design-constraints)
- [Library System Limitations](#library-system-limitations)
- [Unimplemented Features](#unimplemented-features)
- [String Iteration Semantics](#string-iteration-semantics)
- [Stack Slot Assignment (In Progress)](#stack-slot-assignment-in-progress)
- [Code Quality](#code-quality)
- [Store Lifecycle Bugs](#store-lifecycle-bugs)
- [Collection Data Bugs](#collection-data-bugs)

---

## Open Issues — Quick Reference

| # | Issue | Severity | Workaround? |
|---|-------|----------|-------------|
| 22 | Spatial index (spacial<T>) operations not implemented | Low | N/A |
| 24 | Compile-time slot assignment incomplete | Low | No user impact yet |
| 53 | Compile-time intrinsic names not reserved as keywords | Medium | Avoid naming fns `fields`, `debug_assert`, `sizeof`, `assert`, `panic` |
| 54 | `json_items` returns opaque `vector<text>` — no compile-time element type | Low | Accepted limitation; `JsonValue` enum deferred |
| 55 | Thread-local `http_status()` pattern is not parallel-safe | Medium | Use `HttpResponse` struct instead; do not add `http_status()` |
| ~~56~~ | ~~Native codegen: `OpSortVector`, `OpInsertVector`, `OpLengthCharacter` unimplemented in codegen_runtime.rs — generated files fail to compile~~ | ~~Medium (native only)~~ | **FIXED** (0.8.2) |
| ~~57~~ | ~~Native codegen: character variable type is `i32` in generated Rust; `.is_alphanumeric()` call requires `char`~~ | ~~Low (native only)~~ | **FIXED** (0.8.2) |
| ~~44~~ | ~~Empty vector literal `[]` cannot be passed directly as a mutable vector argument~~ | ~~Low~~ | **FIXED** |
| ~~20~~ | ~~`f#next = pos` seek before first open is a no-op~~ | ~~Low~~ | **FIXED** |
| ~~45~~ | ~~`&vector` parameter triggers "never modified" for clear-like ops~~ | ~~Low~~ | **FIXED** |
| ~~49~~ | ~~Integer arithmetic silently wraps on overflow~~ | ~~Medium~~ | **FIXED** (debug builds) |
| ~~47~~ | ~~Invalid UTF-8 in source file silently truncates parsing~~ | ~~Low~~ | **FIXED** |
| ~~48~~ | ~~Runtime `read_to_string` panics on non-UTF-8 file data~~ | ~~Medium~~ | **FIXED** |

---

## Runtime Crashes

### ~~1. Methods returning a new struct record crash inside `var_ref`~~ **FIXED**

**Fixed 2026-03-13.** `parse_single` (parser.rs) was creating `OpCopyRecord(c, d, tp)` as
the self-argument to a method call, leaving `d` uninitialized.  Fix: pass `Var(c)` directly;
`generate_set` gained a branch that emits `ConvRefFromNull + Database + CopyRecord` for
same-type owned-Reference assignment.  Test: `method_returns_new_struct_record` in `tests/issues.rs`.

---

### ~~2. Borrowed-reference pre-init causes runtime crash at database.rs:1462~~ **FIXED**

The `long_lived_int_and_copy_record_followed_by_ref` test in `tests/slot_assign.rs`
(previously described as failing) now passes. Borrowed refs first assigned inside a branch
are correctly pre-initialized by the Option A sub-3 work in `scopes.rs`.
Also verified: `ref_inside_branch_borrowed` in `tests/issues.rs` passes.

---

### ~~3. Polymorphic text methods on struct-enum variants~~ **FIXED 2026-03-13**

Three fixes were applied on 2026-03-13:

1. **`enum_fn` in parser.rs** — collects `extra_call_args`/`extra_call_types` from the
   dispatcher's own `args[1..]` (the `RefVar(Text)` buffer attrs added by `text_return`)
   and forwards them to each variant call via `enum_numbers`.  The `enum_numbers`
   signature was extended to accept `extra_args: &[Value]` and `extra_types: &[Type]`.
   Dispatcher IR result:
   ```
   fn t_5Shape_describe(self:ref(Shape), __work_1:&text) -> text["__work_1"]
     if ... { return t_6Circle_describe(self(0), __work_1(0)); }
     if ... { return t_4Rect_describe(self(0), __work_1(0)); }
   ```

2. **`generate_call` in state.rs** — special case: when forwarding a `RefVar(_)` arg
   as a `Var(v)` with `v` also typed `RefVar(_)`, emit only `OpVarRef(var_pos)` (no
   trailing `OpGetStackText`).  This passes the raw `DbRef` pointer rather than the
   dereferenced text content.

3. **`format_stack_float` in state.rs** — off-by-4 bug: the function pops float(8) +
   int(4) + int(4) = **16 bytes** from the stack but called `string_ref_mut(pos - 12)`
   (the `format_stack_long` offset, which pops only 12 bytes).  Changed to
   `string_ref_mut(pos - 16)` to match `format_float`.  This was the root cause of the
   SIGSEGV — the DbRef was read 4 bytes too low, yielding a garbage pointer.

**Test:** `polymorphic_text_method_on_enum` in `tests/issues.rs` now passes without `#[ignore]`.

---

### ~~4. `v += items` inside a ref-param function does not modify the caller's vector~~ **FIXED**

**Fix (2026-03-13):** Added `assign_refvar_vector` in `parser.rs` (analogous to
`assign_refvar_text`), called from `parse_assign` after `assign_refvar_text`.

**How it works:**
- For `v: &vector<T>` with `v += extra` (non-bracket RHS): emits
  `OpAppendVector(Var(v_nr), extra_expr, rec_tp)` directly.
- `generate_var(v_nr)` for `RefVar(Vector)` emits `OpVarRef + OpGetStackRef(0)`,
  which reads `buf`'s actual DbRef from the `OpCreateStack` temp record — exactly
  what `append_vector` (fill.rs) needs as its `v_r` argument.
- Bracket-form `[elem]` RHS produces `Value::Insert`; `assign_refvar_vector` skips
  those (returns false) so `parse_block` expands them via the existing
  `OpNewRecord / OpSetInt / OpFinishRecord` path (which already works for ref-params).
- `find_written_vars` recognises `OpAppendVector(Var(v_nr), ...)` as a write to
  `v_nr` via the `stack_write` extension (name starts with `"OpAppend"`-check).

**Test:** `ref_param_append_bug` in `tests/issues.rs` now passes.

---

### ~~5. Appending a scalar to a vector struct field that starts empty has no effect~~ **FIXED**

`b.items += 1` where `items` is a `vector<T>` field now works.
Fix: in `parse_assign` (parser.rs), when `var_nr == u16::MAX` (field access), `op == "+="`,
`f_type == Type::Vector`, and the RHS is a scalar element (not an `Insert`), route through
`new_record` with `is_field = true` — the same path as `b.items += [1]` — which uses
`OpNewRecord`/`OpFinishRecord` to allocate the element directly in the struct's field.
Tests: `vec_field_append_scalar` and `vec_field_append_bracket_*` in `tests/issues.rs`.

---

### ~~27. `16-parser.loft` runtime crash: `store_nr=60` in `set_int`~~ **FIXED 2026-03-15**

**Symptom (fixed):** `wrap::last` and `wrap::dir` (at `16-parser.loft`) crashed with:
```
thread 'last' panicked at src/database/allocation.rs:104:
index out of bounds: the len is 8 but the index is 60
```

**Root cause:** The loft parser's `Code.finish_define()` contained `self.cur_def = null`.
The `null` keyword in loft is parsed as `Value::Null` with `Type::Null`. For binary-operator
chains, `handle_operator` calls `convert()` to turn `Type::Null` into a typed null constant
(e.g. `Value::Call(OpConvIntFromNull, [])` for integers). But for simple scalar field
assignments (`field = null`), `convert()` was never called in `parse_assign_op`.

Without the conversion, `generate(Value::Null)` emitted **no bytecode** (returns `Type::Void`).
The `OpSetInt` operator (which pops a 12-byte DbRef then a 4-byte integer) therefore popped its
integer argument from the wrong stack bytes — reading across the return-address bytes and into
the DbRef itself. The bytes `[0x3C, 0x00]` (ASCII `<`) happened to map to `store_nr=60`,
causing the out-of-bounds panic.

**Fix (`src/parser/expressions.rs`, `parse_assign_op`):** After parsing the RHS, if
`s_type == Type::Null` and `op == "="`, call `self.convert(code, &Type::Null, f_type)` to
replace `Value::Null` with the appropriate typed-null constant before `towards_set` processes it.

**Note:** `16-parser.loft` now passes (PROBLEMS #42 fixed 2026-03-16).

---

### 28. ~~`validate_slots` panic: same-name variables in sequential blocks~~ **FIXED**

`find_conflict()` now exempts pairs where both variables have the same name and the
same stack slot — these are sequential-block reuses of one logical variable, not
runtime conflicts.  Both same-name (`n` / `n`) and different-name (`a` / `b`) cases
in sequential blocks pass without panicking.

Tests `sequential_blocks_same_varname_workaround` and `sequential_blocks_different_varnames`
in `tests/issues.rs` cover this.

**Note:** This fix only applies to same-name pairs. A broader case (Issue 29) remains
unfixed where two variables with *different* names share a slot and have overlapping
`first_def`/`last_use` intervals, even though they are never simultaneously live at runtime.

---

### ~~29. `validate_slots` false positive: different-name reference variables in the same function~~ **FIXED 2026-03-13**

**Symptom:** In a large function that reuses a reference-typed variable (e.g. `f` as a file
handle) across many sequential `{ }` blocks, and later introduces a second reference-typed
variable with a different name (e.g. `c` for a `vector<text>`), `validate_slots` panics:

```
Variables 'f' (slot [1000, 1012), live [237, 1699]) and 'c' (slot [1000, 1012),
live [1539, 1540]) share a stack slot while both live in function 'n_main'
```

Both `f` and `c` are assigned the same 12-byte slot (both are reference types). Their live
intervals overlap: `f.first_def=237 < c.last_use=1540` and `c.first_def=1539 < f.last_use=1699`,
satisfying the `find_conflict` overlap condition. In reality they are never live at the same
time — `f` is only active inside its `{ }` blocks, which are all disjoint from the use of `c`.

**Root cause:** `compute_intervals` stores a global `first_def`/`last_use` per variable,
spanning the entire function regardless of block scope. For a variable reused across
many sequential blocks, `last_use` is the bytecode position of its final block, which
can be far past the introduction of other variables that share the same slot.

Issue 28's fix (exempt same-name/same-slot pairs) does not help here because `f` and `c`
have different names.

**Workaround:** Reorder the code so that all uses of the first variable finish before the
second is introduced. In `tests/scripts/11-files.loft`, the `c = ...lines()` call was moved
to the very end of `fn main()` to ensure `c.first_def > f.last_use`.

**Fixed 2026-03-13:** `find_conflict()` in `variables.rs` already uses interval-based overlap
checking (`left.first_def <= right.last_use && right.first_def <= left.last_use`) and includes
an exemption for same-name/same-slot pairs.  All Issue 29 tests pass.  The "differently-named"
case is handled because the overlap check is precise: if their live intervals truly do not
overlap, `find_conflict` correctly finds no conflict regardless of whether names match.

**Relationship to Issue 24:** Full per-block liveness (Step 3 of [ASSIGNMENT.md](ASSIGNMENT.md)) would make
the interval tracking exact; the current whole-function range is a conservative approximation
but is sufficient in practice for the known test cases.

---

## Parser / Lexer Bugs

### 53. Compile-time intrinsic names not reserved as keywords

**Symptom:** The parser special-cases several function-shaped names in `parse_call`
(`sizeof`, `assert`, `panic`, `log_info/warn/error/fatal`) and `parse_in_range` (`rev`).
Because these names are not in the `KEYWORDS` array in `src/lexer.rs`, a user can define
`fn sizeof(...)` or `fn assert(...)`.  The intrinsic handling in `parse_call` takes
precedence over any user definition — so the user's function is never called through the
normal call path and becomes unreachable dead code.  `match` was similarly not in
KEYWORDS when it shipped in 0.8.0 (the documented list in COMPILER.md was never updated).

**Forward-compatibility risk:** Two upcoming 0.9.0 features introduce new intrinsic names:
- `fields` (A10 — field iteration): a common English word; user code written today could
  easily define `fn fields(s: Config) -> vector<text>` and rely on it.  When A10 lands, that
  call site silently stops dispatching to the user's function.
- `debug_assert` (A2.3 — release mode): behaves identically to `assert` at the call site;
  must be claimed before any user code adopts the name.

**Severity:** Medium — no crash; but the intrinsic always silently wins, so the user's
function becomes dead code with no diagnostic.  The forward-compatibility risk for `fields`
and `debug_assert` is the primary concern.

**Workaround:** Do not name functions or variables `sizeof`, `assert`, `panic`, `fields`,
or `debug_assert`.

**Fix path:**
1. Add `match`, `sizeof`, `assert`, `panic`, `fields`, `debug_assert` to the `KEYWORDS`
   array in `src/lexer.rs`.  These six names have parse-time semantics (file+line injection,
   compile-time type resolution, loop unrolling, or assert elision) that cannot be expressed
   by a regular function definition.
2. Update the KEYWORDS list in `doc/claude/COMPILER.md` to reflect the full set.
3. Add a parse-error test: `fn sizeof(x: integer) -> integer { x }` must produce a single
   diagnostic naming `sizeof` as a reserved keyword.

Names intentionally left as identifiers (not promoted to keywords): `log_info/warn/error/fatal`
(prefixed, low collision risk), `parallel_for` (highly specific), `rev` (likely to become a
genuine stdlib function for vector reversal).

**Files:** `src/lexer.rs`, `doc/claude/COMPILER.md`
**Effort:** Small
**Target:** Before 0.9.0 — `fields` and `debug_assert` must be claimed before those
features land; the rest closes a pre-existing gap.

---

### 6. ~~Uppercase hex literals are rejected~~ **FIXED**

`0xFF`, `0x2A` etc. now accepted. Both `get_number()` and `hex_parse()` already handled uppercase; tests verified.

---

### 7. ~~Open-start slice syntax `s[..n]` is not supported~~ **FIXED**

`s[..n]` for text and `v[..n]` for vectors now work; `parse_in_range` detects leading `..` and defaults start to 0.

---

### 8. ~~Calling a method on a constructor expression is rejected~~ **FIXED**

`Pt{x:3,y:4}.dist2()` now works. Fixed two-part type mismatch in `parse_object` and `convert` (parser.rs). Test: `method_on_constructor` in `tests/objects.rs`.

---

### 9. Nested `\"` inside format expressions is not supported — **FIXED 2026-03-14**

**Symptom:** Using `\"` inside a `{...}` format expression in a string literal caused
`Error: Dual definition of ...`.

**Example:**
```loft
// FIXED — \" inside the format expression now works
s = "{\"hello\"}";        // s == "hello"
s = "{\"hello\":5z}";     // Unexpected formatting type: z
```

**Fix:** `src/lexer.rs` — added `in_format_expr: bool` flag to `Lexer`. When `{` opens
a format expression, `in_format_expr` is set; when `}` closes it, the flag is cleared.
While `in_format_expr` is true, `"` and `\"` dispatch to the new `string_nested()` method
instead of calling `string()` (which would close the outer string). `string_nested()`
scans the nested literal content and returns it as `LexItem::CString` without touching
the lexer mode.

**Tests:** `tests/format_strings.rs` — `string_literal_no_specifier`,
`string_literal_with_width`, `string_literal_bad_specifier_after_width`,
`string_literal_bare_bad_specifier`.

---

### 10. ~~`{expr}` in string literals — use-before-assignment panics in byte-code generator~~ **FIXED**

**Root cause:** In `parse_assign`, `vars.defined(v_nr)` was called *before*
`lexer.has_token("=")` confirmed that an actual assignment was present.  Any bare
`Value::Var` in expression position (e.g. `{cd}` inside a format string) was
therefore incorrectly marked as defined, even when `cd = 5` came *after* the string
literal in the source.  The byte-code generator then saw `stack_pos == u16::MAX` and
panicked.

**Fix (2026-03-15):** Moved `vars.defined(v_nr)` inside the `has_token("=")` block
in `parse_assign`.  The variable is now marked defined only when the `=` token is
actually present.  Also added `vars.defined(count)` in the `#count`/`#first` branches
of `iter_op` so that lazily-created loop-counter variables (e.g. `e#count`) are
correctly marked defined.

**Tests:** `format_string_use_before_assign`, `format_string_use_after_assign`,
`format_string_loop_count` in `tests/format_strings.rs`.

---

### 11. ~~Field-name overlap between two structs causes wrong index range results~~ **NOT A BUG**

`determine_keys()` is type-scoped, so field offsets for identically-named fields in
different structs are computed independently.  Range query boundary semantics are also
correct (descending sort key ordering).  Test `field_name_overlap_range_query` passes.

---

## Library System Limitations

### 12. ~~`use` statements must appear before all definitions~~ **FIXED**

`parse_file` already emits a `Fatal` diagnostic; no code change was needed.

---

### ~~13. Unqualified access to library definitions is not supported~~ **FIXED 2026-03-16**

`use mylib::*` imports all names from `mylib` into the current scope;
`use mylib::Point, add` imports specific names. Local definitions shadow imported
names (no error). Implemented in T1-2; three tests in `tests/imports.rs`.

---

### 14. ~~Cannot add methods to a library type from an importing file~~ **FIXED**

`get_fn` in `data.rs` now falls back to `source_nr(self.source, name)` when the struct's source doesn't define the method. Test: `fn shifted(self: testlib::Point, ...)` in `tests/docs/17-libraries.loft`.

---

### 15. ~~`pub` on struct fields causes a parse error~~ **FIXED**

`parse_struct` already silently consumes the `pub` keyword; no code change needed.

---

## Unimplemented Features

### 16. ~~`for n in range { expr }` inside a vector expression is not supported~~ **FIXED**

`[for n in 1..7 { n * 2 }]` now works via `parse_vector_for` in `parser.rs`. Tests: `for_comprehension` and `for_comprehension_if` in `tests/vectors.rs`.

---

### 17. ~~Reverse iteration on `sorted<T>` is not implemented~~ **FIXED 2026-03-14**

**Symptom:** Trying to iterate a sorted collection in reverse order panicked.

**Fix:** Three-part change:
1. `src/parser/collections.rs` — `parse_in_range()` recognises `rev(sorted_var)` (no `..`):
   consumes the closing `)`, sets `self.reverse_iterator = true`.  The flag is also
   cleared in the first-pass early return of `iterator()` to prevent it from leaking
   across passes.  `fill_iter()` reads the flag on both its calls (OpIterate + OpStep)
   and ORs bit 64 into the `on` byte before resetting the flag.  `Parser` gains a
   `reverse_iterator: bool` field.
2. `src/vector.rs` — added `vector_step_rev()` which mirrors `vector_step()` but
   decrements the element index.  Any position `>= length` (used by `iterate()` as the
   "not started" sentinel for reverse) initialises to `length - 1`.  Also fixed a
   pre-existing overflow in `sorted_find()` when the sorted collection is empty
   (`sorted_rec == 0` or `length == 0` now return `(0, false)` early).
3. `src/fill.rs` — `step()` for `on & 63 == 2` (sorted): calls `vector_step_rev`
   when `reverse` is set; stops when `pos == i32::MAX` (returned by `vector_step_rev`
   when the beginning has been passed).

**Tests:** `sorted_reverse_iterator`, `sorted_reverse_empty` in `tests/vectors.rs`;
reverse section added to `tests/docs/10-sorted.loft`.

---

### 18. ~~Writing a vector to a binary file produces 0 bytes~~ **FIXED**

`write_file` in `state.rs` now handles `Parts::Vector` types. Test: `array_write` in `tests/file-system.rs`.

---

### 19. ~~`f#size = n` (file truncate / extend) is not implemented~~ **FIXED**

Two parser/state bugs fixed (wrong lookup name; format not updated after create). All 6 file-size tests pass.

---

### ~~20. `f#next = pos` (file seek) only works after the file is already open~~ **FIXED**

**Fixed:** `seek_file()` now stores the position in `#next` when the file is not yet
open, so the first read/write applies the pending seek automatically.

---

### ~~21. Command-line arguments cannot be passed to `fn main()`~~ **FIXED 2026-03-13**

`Stores::text_vector(&[String])` builds a `vector<text>` DbRef from a Rust string slice.
`State::execute_argv()` detects a single `Type::Vector` attribute on `main` and pushes the DbRef before the return address.
`main.rs` collects trailing positional arguments into `Vec<String>` and calls `execute_argv`.
Test: `wrap::main_argv`.

---

### 22. Spatial index operations are not implemented

**Pre-gate fix (2026-03-15):** `spacial<T>` in any field or variable type now emits a
compile-time error:
```
spacial<T> is not yet implemented; use sorted<T> or index<T> for ordered lookups
```
Both first-pass and second-pass are covered; no program can reach the runtime `Not implemented`
panics via `spacial<T>` anymore.  Test: `spacial_not_implemented` in `tests/parse_errors.rs`.

**Remaining work:** The full implementation (insert, lookup, copy, remove, iteration) is still
missing.  Best way forward: implement one operation at a time in `database.rs` and `fill.rs`,
starting with iteration, then remove, then copy.  The spacial index structure (radix tree or
R-tree) is already allocated in the schema; the iteration traversal is the main missing piece.

---

## String Iteration Semantics

### ~~23. `c#index` in `for c in text` returns post-advance offset instead of pre-advance~~ **NOT A BUG**

`parser.rs` lines 317-318 already save the pre-advance offset into `{id}#index` before emitting `OpTextCharacter` and advancing the pointer. The entry was stale. Verified by the `char iter indices` assertion in `tests/scripts/14-formatting.loft`.

---

## Stack Slot Assignment (In Progress)

### 24. Full compile-time slot assignment not yet implemented

**Current:** Stack slot positions are determined at codegen time by `claim()` in
`state.rs`. The final two steps of the planned P2 pass are missing:
- `assign_slots()` — compute optimal positions using precomputed live intervals
- Remove `claim()` calls and `copy_variable()` once the above is done

**Impact:** Non-optimal stack usage; potential conflicts detected by `validate_slots`
in debug builds.

**Best way forward:** Implement `assign_slots()` in `variables.rs` as a greedy
interval-graph colouring: sort variables by `first_def`, assign each to the lowest
slot position not occupied by a live variable of incompatible type. Wire it into
`scopes::check` after `compute_intervals`. Once all tests pass with `assign_slots`,
remove `claim()` from `state.rs`.

**Details:** [ASSIGNMENT.md](ASSIGNMENT.md) §"P2 — Full slot assignment pass".

---

### 26. ~~`vector_db` panics with `var_nr=65535` when appending struct elements to a vector field~~ **FIXED**

Added `vec == u16::MAX` guard in `vector_db` (parser.rs). Test: `tests/docs/19-threading.loft`.

---

### ~~31. `v += [struct_var]` appends empty element instead of copying struct~~ **FIXED 2026-03-13**

**Root cause:** In `new_record` (parser.rs), the branch for `Type::Reference` elements inside a vector literal had three cases: `Value::Insert` (inline literal — correct), `is_field` (field access — emits `OpCopyRecord`), and `else` (variables, calls — just pushed the expression without copying). The `else` case created a new empty element slot then discarded the source DbRef without copying the struct data into the slot, leaving all float/reference fields at their default null/NaN values.

**Fix:** Collapsed the `is_field` and `else` branches into a single `else` that always emits `OpCopyRecord(src, Var(elm), type_nr)`. Both field accesses and variable references evaluate to a DbRef that `OpCopyRecord` can read from.

**Tests:** `tests/docs/17-libraries.loft` (append via var + inline literal), `tests/scripts/07-structs.loft` (vectors of structs).

---

### ~~32. `v += other_vec` replaces vector instead of appending~~ **FIXED 2026-03-13**

**Root cause:** `v += other_vec` where the RHS is an existing `vector<T>` variable (not a bracket literal `[...]`) was not dispatched to `OpAppendVector`. For a local variable LHS, `towards_set` emitted `Set(v, Var(other_vec))` (reassignment). For a field LHS (`bx.pts += more`), the scalar-field-append path in `parse_assign` was triggered, treating the whole vector as a single element to append.

**Fix:** Added a new guard in `parse_assign` (before the scalar-field-append check): when `f_type` is `Vector`, `op == +=`, and the RHS type is also `Vector` (not `Insert`), emit `OpAppendVector(lhs_expr, rhs_expr, rec_tp)` directly. Works for both variable and field LHS since both evaluate to a DbRef.

**Tests:** `tests/docs/17-libraries.loft` (field vector-to-vector append), `tests/scripts/09-vectors.loft`.

---

### 30. ~~`for c in enum_vector` loops forever~~ **FIXED**

**Fixed 2026-03-13** (`src/fill.rs::get_enum`): past-end `OpGetVector` returns null
`DbRef`; `get_byte(rec=0)` returns `i32::MIN`, which cast to `u8` gave `0` (valid
first variant) instead of the sentinel `255` that breaks the loop.  Fix: map
`i32::MIN → 255u8` before pushing.  Tests: `tests/scripts/08-enums.loft`.

---

## ~~B-Tree Collection Bugs (found by stress tests, 2026-03-14)~~ ALL FIXED 2026-03-14

### ~~33. `sorted` filtered loop-remove gives wrong sum for large N~~ **FIXED 2026-03-14**

No actual bug was present — the sorted filtered loop-remove was never broken.
`sorted_filtered_remove_large` (N=100) and the expanded stress test section E pass.

---

### ~~34. `index` key-null removal leaves 1 record for large N~~ **FIXED 2026-03-14**

**Root cause:** In `tree::remove()` (`src/tree.rs`), when the last remaining element is
removed, `remove_iter` correctly returns `new_top = 0` (empty tree), but the function
only updated the root pointer `if new_top > 0`.  The root was never cleared to 0, leaving
a phantom element accessible via the stale root pointer.

**Fix:** Always update the root pointer in `tree::remove`, even when `new_top == 0`.

**Tests:** `index_key_null_removes_all` in `tests/vectors.rs` (N ∈ {1,2,3,10,50,100});
`tests/scripts/16-stress.loft` section A (N=100 key-null).

---

### ~~35. `index` loop-remove panics "Unknown record" for large N~~ **FIXED 2026-03-14**

**Root cause (two-part):**
1. In `fill_iter()` (`src/parser/collections.rs`), for Index collections (`on=1`), `loop_db_tp` was
   set to `arg = database.fields(known)` (the byte offset of the RB-tree metadata field)
   instead of `known` (the Index type index).  The OpRemove bytecode therefore received a
   field-offset value as its `tp` argument.
2. In `state::remove()` (`src/fill.rs`) for `on==1`, `tp` was used in
   `self.database.size(tp)` (treating the offset as a type index) to create the `DbRef`
   for `tree::next/previous`.  The wrong `.pos` value caused `tree::next` to read the
   right-child pointer from the wrong offset, returning a garbage record number.  The next
   `step()` call then called `store.valid()` on that garbage record, hitting
   `claims.contains()` → "Unknown record" panic.

**Fix:**
- `fill_iter()`: store `known` (type index) in `loop_db_tp` for Index; the bytecode
  `arg` (fields offset) for OpIterate/OpStep is unaffected.
- `state::remove()` for `on==1`: compute `let fields = self.database.fields(tp)` and use
  it instead of `self.database.size(tp)` for tree navigation; pass `tp` (now the correct
  type index) to `database.remove`.

**Tests:** `index_loop_remove_small` (N=3) and `index_loop_remove_large` (N=20) in
`tests/vectors.rs`; `tests/scripts/16-stress.loft` section A2 (N=100 loop-remove).

---

### ~~36. Enum debug display panics with index out of bounds~~ **FIXED 2026-03-15**

**Symptom:** Running any loft program that uses an enum field in debug mode (`file_debug`
or `LOFT_LOG=full`) panicked at `src/database/format.rs`:

```
index out of bounds: the len is 5 but the index is 7
```

**Root cause:** In `database/format.rs`, the debug display for `Parts::Enum` variants computed
`v as usize - 1` unconditionally before checking that `v > 0`, causing unsigned subtraction
overflow when `v == 0`. When `v > 0` but out of the enum's range (e.g. a garbage byte read
from an uninitialized region), the unchecked index caused a panic.

**Fix:** Added a bounds check around both the enum name lookup and the `tp_nr` lookup:
- `v <= 0` → display `"null"` (was already handled but `idx` was computed before the check)
- `v as usize - 1 >= vals.len()` → display `"?"` instead of panicking

**Triggered by:** `wrap::file_debug` running `tests/docs/13-file.loft` in debug mode.

---

## Web Services Design Constraints

### 54. `json_items` returns opaque `vector<text>` — no compile-time element type
**Severity:** Low — accepted design limitation
**Description:** `json_items(body)` parses a JSON array and returns the element bodies as
`vector<text>`.  There is no way for the compiler to verify that the caller's parse function
(e.g. `User.from_json`) receives a valid JSON object body rather than an arbitrary string.
A parse error at runtime produces a partial zero-value struct, not a diagnostic.
**Workaround:** Validate the HTTP response status before parsing (`if resp.ok()`).
**Fix path:** Deferred.  A `JsonValue` enum (covering Object, Array, String, Number, Boolean,
Null variants) would give compile-time structure, but the design cost is high.
**Effort:** Very High (deferred)
**Target:** 1.1+
**See also:** [WEB_SERVICES.md](WEB_SERVICES.md)

---

### 55. Thread-local `http_status()` is not parallel-safe
**Severity:** Medium — design trap; do not introduce this API
**Description:** An `http_status()` function returning the status of the most recent HTTP
call as a thread-local integer (the pattern used by C's `errno`) is tempting but incorrect
in loft's parallel execution model.  A `parallel_for` worker calling `http_get` would
corrupt the thread-local of the calling thread.
**Fix path:** Return an `HttpResponse` struct directly from all HTTP functions.  The status
is a field on the returned value, not global state.  See WEB_SERVICES.md Approach B.
**Effort:** N/A — this is a design constraint, not a bug to fix.  Simply do not add `http_status()`.
**Target:** Avoided by design

---

## Code Quality

### 25. ~~Dead Option-B helpers in variables.rs~~ **ALREADY CLEAN**

No dead functions found; `first_def` is a struct field only.

---

## Store Lifecycle Bugs

### ~~37. `OpFreeRef` emitted in forward declaration order → LIFO store-free panic~~ **FIXED 2026-03-15**

**Fixed 2026-03-15.** `scopes.rs::variables()` was iterating `self.var_scope` (a
`BTreeMap<u16, u16>`) in ascending variable-number order, causing `get_free_vars()` to emit
`OpFreeRef` in forward declaration order. `database::free()` enforces LIFO (most-recently-allocated
store freed first), so two or more owned refs in the same scope → panic.

**Root cause detail:** Simply reversing the BTreeMap order (`res.reverse()`) was
insufficient: work-ref backing variables (`__ref_N` generated for `if/else` branches) have
higher var_nr but lower `first_def` (they are pre-initialized earlier in bytecode via
`ConvRefFromNull`), so ascending or descending var_nr order both produced LIFO violations in
different code paths.

**Actual fix:** Added `var_order: Vec<u16>` to the `Scopes` struct. Every time a variable is
first inserted into `var_scope` (in `scan_set()` and in the `scan_if()` pre-init loop), it is
also pushed onto `var_order`. `variables()` now iterates `var_order` in **reverse** (most-recently
inserted first = last allocated = freed first), giving a free order that always matches the
inverse of `ConvRefFromNull` allocation order regardless of var_nr assignment.

**Tests:** `lifo_store_free_two_owned_refs`, `lifo_store_free_three_owned_refs` (both in
`tests/issues.rs`). Previously failing: `structs`, `enums`, `vectors`, `collections`,
`binary`, `binary_ops`, `text`, `files`, `map_filter_reduce`, `script_threading`,
`threading` — all now pass.

---

### ~~38. T0-1 fix regression: `sorted`/`hash`/`index` key-null removal silently broken~~ **FIXED 2026-03-15**

**Fixed 2026-03-15.** In `parse_assign_op` (`src/parser/expressions.rs`), the `convert()`
call that substitutes typed-null constants for scalar field assignments was running
unconditionally for all null assignments. For reference-typed LHS (collection element refs),
`convert()` replaced `Value::Null` with `Value::Call(OpConvRefFromNull, [])`, causing
`towards_set_hash_remove`'s `*val == Value::Null` guard to fail. The element was then silently
not removed.

**Fix:** Added a type guard so `convert()` only runs for non-reference, non-collection types:
```rust
if s_type == Type::Null
    && op == "="
    && !matches!(
        f_type,
        Type::Reference(_, _) | Type::Enum(_, true, _)
            | Type::Vector(_, _) | Type::Sorted(_, _, _)
            | Type::Hash(_, _, _) | Type::Index(_, _, _)
    )
{
    self.convert(code, &Type::Null, f_type);
}
```

**Tests:** `sorted_key_null_removes_entry`, `hash_key_null_removes_entry`,
`index_key_null_removes_entry` in `tests/issues.rs`. `wrap::stress`, `wrap::dir`,
`wrap::loft_suite` now pass.

---

### ~~41. Inline ref-returning calls leak their store → LIFO panic~~ **FIXED 2026-03-15**

**Root cause:** `parse_part()` synthesised anonymous work-ref variables (`__ref_1`,
`__ref_2`, …) for inline ref-returning chains such as `p.shifted(1.0, 2.0).x`.
These were null-initialised with `OpNullRefSentinel` (no real store) inserted after
the first user statement in the function.  Crucially, that insertion point was often
BEFORE the `Set(p, null)` null-init for body variables declared later in the function.
`scan_set` then saw the work-refs before `p` in the block, so reversed `var_order`
freed `p` (store 2) before the work-refs (stores 3, 4 …) — a LIFO violation.

**Fix (2026-03-15):** Four coordinated changes:
1. **`OpNullRefSentinel`** (`default/01_code.loft`, `src/fill.rs`): new opcode that
   pushes `DbRef{store_nr:u16::MAX}` without allocating a database store.  Used for
   work-ref null-inits so they are harmless no-ops if the variable is never assigned.
2. **Sentinel guards** (`src/database/allocation.rs`): `free()` and `valid()` return
   early when `store_nr == u16::MAX`.
3. **`gen_set_first_ref_null`** (`src/state/codegen.rs`): emits `OpNullRefSentinel`
   instead of `OpConvRefFromNull` for inline-ref temporaries.
4. **Null-init placement** (`src/parser/expressions.rs`): inline-ref null-inits are
   now inserted immediately BEFORE the statement that first assigns each work-ref
   (found by recursively searching the block for `Set(r, _)` nodes), rather than after
   the first user statement.  This places the work-ref in `var_order` AFTER any body
   variable whose store precedes it, so reversed `var_order` frees the work-refs BEFORE
   those variables — satisfying LIFO for all nesting depths.
5. **`Function::copy/append`** (`src/variables.rs`): preserve `inline_ref_vars` across
   the parse→definition→scopes→codegen pipeline so the work-ref set is never lost.

**Tests:** `tests/issues.rs` (`inline_ref_call_field_access`,
`inline_ref_call_double_chain`), `tests/docs/17-libraries.loft` (removed from
`SUITE_SKIP` in `tests/wrap.rs`).

---

## Collection Data Bugs

### ~~39. `v += other_vec` shallow copy: text/ref fields in appended struct elements become dangling~~ **FIXED 2026-03-15**

**Root cause:** `Stores::vector_add()` in `src/database/structures.rs` used
`copy_block` to append element bytes from the source vector to the destination vector.
This raw byte copy transferred text-field slot-index values as-is without calling
`copy_claims()` to duplicate the string records. Both source and destination vectors then
shared the same string record indices; when either was freed, `remove_claims()` deleted
those records and the other vector's text fields became dangling → double-free / "Unknown
record N" panic.

**Fix:** In `vector_add()` (`src/database/structures.rs`), after the `copy_block`
(or cross-store `copy_block_between`), iterate each appended element and call
`copy_claims(src_elem, dst_elem, known)` to create independent copies of all string
records and sub-structures in the destination store. This mirrors the approach already
used by `copy_claims_seq_vector()`.

**Tests:** `vec_add_text_field_deep_copy`, `vec_add_text_field_non_empty_dest` in
`tests/issues.rs`.

---

### ~~40. `index<T>` as struct field: `OpCopyRecord` and `OpClear` panic~~ FIXED 2026-03-15

**Fix:** Added `collect_index_nodes` helper (in-order RB-tree traversal) and
`copy_claims_index_body` to `allocation.rs`.  `copy_claims` now calls
`copy_claims_index_body` for `Parts::Index`; `remove_claims` has an inline Index arm
that iterates collected nodes, calls `remove_claims` for each element's sub-fields, and
deletes the node records.  Three regression tests in `tests/issues.rs`:
`index_field_copy_claims_via_vector`, `index_field_copy_claims_text_elements`,
`index_field_remove_claims_on_reassign`.

---

### ~~42. `16-parser.loft`: `generate_call` size mismatch for mutable Reference argument~~ **FIXED 2026-03-16**

**Symptom (fixed):** Compiling `tests/docs/16-parser.loft` triggered a `debug_assert` in
`generate_call` (`src/state/codegen.rs`):
```
generate_call [t_4Code_define]: mutable arg 0 (data: Reference(244, [])) expected 12B on stack but generate(Var(6)) pushed 4B
```
`wrap::last` and `wrap::parser_debug` were `#[ignore]`; `16-parser.loft` was listed in
`SUITE_SKIP`.

**Root cause:** In `lib/code.loft`, `Code.define()` contained `self.def_names[name] = res`
where `def_names: hash<Definition[name]>` and `res: i32`.  This stored a 4-byte integer
into a hash slot that expected a full 12-byte Reference, causing the caller-side size to
disagree with the callee's `data: reference` parameter (12B).

Three additional bugs were uncovered once `define()` compiled correctly:
1. `get_type()` read `def_names[name].typedef` instead of the actual `definitions[nr].typedef`, returning stale/zero data.
2. `structure()` in `lib/parser.loft` called `type_def()` which internally called `define()` and `finish_define()`, resetting `cur_def` to null before the following `argument()` call — so struct fields were not registered.
3. `object()` in `lib/parser.loft` had the loop continuation condition inverted (`!self.lexer.test("}}") { break }` instead of `self.lexer.test("}}") { break }`), causing the parser to exit after one field in any struct literal with multiple fields.

Additionally, `define()` used `if self.def_names[name] != null` which generates `ConvRefFromNull()` — a store-allocating opcode — with no matching `FreeRef`, leaking one store per call and eventually causing a LIFO store-free panic.

**Fix (2026-03-16):**
- `lib/code.loft`: Changed `self.def_names[name] = res` → `self.def_names += [Definition { name: name, nr: res }]` so a full Definition is stored.
- `lib/code.loft`: Changed `get_type()` to look up `definitions[def_names[name].nr].typedef` for the canonical typedef value.
- `lib/code.loft`: Changed `define()` null check from `if self.def_names[name] != null` to `nr = self.def_names[name].nr; if nr != null` — integer comparison avoids the store-allocating `ConvRefFromNull`.
- `lib/parser.loft`: `structure()` saves `struct_nr = self.code.define(...)` and restores `self.code.cur_def = struct_nr` after each `type_def()` call.
- `lib/parser.loft`: `object()` corrected the loop condition from `!self.lexer.test("}}") { break }` to `self.lexer.test("}}") { break }`.
- `tests/wrap.rs`: Removed `16-parser.loft` from `SUITE_SKIP`; re-enabled `wrap::last`.

### ~~43. `text_var += text_param` inside `if` inside `for` panics "Incorrectly code Insert not rewritten"~~ **FIXED 2026-03-15**

**Symptom:** Any function containing `text_var += text_param` (text append with a variable
parameter, not a literal) inside an `if` branch inside a `for` loop caused a codegen panic:
```
thread panicked at src/state/codegen.rs: Incorrectly code Insert not rewritten
```
The canonical example is a `join` implementation using `#first`:
```loft
for p in parts {
    if !p#first { result += sep; }  // ← panicked here
    result += p;
}
```

**Root cause:** `parse_append_text` in `expressions.rs` represents `text_var += text_param`
as `Value::Insert([OpAppendText(...)])` — a list of void ops — when appending to an existing
variable.  When this `Value::Insert` appeared as the body of an `if` branch, it ended up as
the `t_val` of a `Value::If` node.  The `scopes.rs` `scan()` function hit the `_ => val.clone()`
arm (no `Value::Insert` case), so the Insert was not scanned for variable remapping.  It then
reached `codegen.rs generate()`, which had `Value::Insert(_) => panic!(...)`.

**Fix (2026-03-15):**
- `src/scopes.rs` `scan()`: added `Value::Insert(ops)` arm that recursively scans each op so
  variable remapping is correctly applied.
- `src/state/codegen.rs` `generate()`: replaced the panic with sequential generation of the
  ops inside the Insert, returning `Type::Void`.  This is the correct semantics: an Insert is
  always a list of void side-effect operations.

**Tests:** `default/03_text.loft` `join` function; `tests/scripts/03-text.loft` join assertions.

---

### 44. Empty vector literal `[]` cannot be passed directly as a mutable vector argument

**Symptom:** Passing `[]` directly as a function argument where the parameter is a mutable
vector (`vector<T>`) fails at compile time or produces incorrect codegen:
```loft
assert(join([], "-") == "", ...);  // ← fails
```
In a debug build, the `generate_call` debug assertion fires:
```
mutable arg 0 (parts: Vector(...)) expected 12B on stack but generate(Insert([Null])) pushed 0B
```

**Root cause:** `parse_vector` in `expressions.rs` returns early with `Value::Insert([Null])`
for an empty `[]` literal when the target is not an existing variable (`is_var = false`).  This
path is reached when `[]` appears as a function call argument, because expression parsing uses
`Type::Unknown(0)` as the initial type context.  With an unknown element type, no temporary
variable is created and no `vector_db` initialisation ops are emitted.  The `Value::Insert([Null])`
carries no stack presence, so the mutable-arg handler finds 0 bytes instead of the 12-byte
`DbRef` it expects.

By contrast, `my_vec = []` in an assignment statement works: on the second pass `my_vec` already
has an inferred type, and `create_vector` (called from `parse_assign_op`) inserts the correct
`vector_db` initialisation ops into the Insert.

**Workaround:** Assign the empty vector to a named variable first; the type is then inferred
from the subsequent call, and the second pass initialises the variable correctly:
```loft
join_empty = [];
assert(join(join_empty, "-") == "", ...);
```

**Fix path:** In `parse_vector`, when the early-return `else` branch is taken (empty `[]`,
not a Var, not a field), create a unique temporary variable, call `vector_db` to initialise it,
push `Value::Var(vec)` as the block result, and wrap in `v_block` — matching what the non-empty
path does when `block = true`.  The difficulty is that `assign_tp` (the element type) is
`Type::Unknown(0)` at this point; `vector_db` must either tolerate Unknown gracefully on the
second pass or this path must be deferred until the call-site type is known.

**Effort:** Medium (parser change; requires careful handling of the Unknown element type)

---

### ~~46. Block expression `{ ... }` as match arm body~~ **FIXED 2026-03-17**

**Symptom (fixed):** Using a block expression as a match arm body crashed at runtime:
```loft
match x {
    1 => { 10 + 1 },   // segfault
    _ => 0
}
```

**Workaround:** Use parentheses or a function call instead of a block:
```loft
match x {
    1 => (10 + 1),      // works
    _ => 0
}
```

**Root cause:** The expression parser sees `{` and starts parsing a block, but
the closing `}` is ambiguous — it could be the block's end or the match's end.

**Impact:** Low — block bodies in match arms are uncommon; parentheses or
function calls achieve the same result.

---

### ~~45. `&vector` parameter annotation triggers "never modified" warning for clear-like operations~~ **FIXED**

**Fixed:** `find_written_vars` now recognizes `OpClearVector`, `OpInsertVector`, and
`OpRemoveVector` as mutations to the first argument, suppressing the false warning.

---

### ~~47. Invalid UTF-8 in source file silently truncates parsing~~ **FIXED**

**Fixed:** Both `next()` code paths in `lexer.rs` now handle `Some(Err(e))` and emit
a `Fatal` diagnostic: `"Cannot read line N — is the file valid UTF-8? (error)"`.
Parsing stops cleanly with a meaningful message.

---

### ~~48. Runtime `read_to_string` panics on non-UTF-8 file data~~ **FIXED**

**Fixed:** `get_file_text()` in `state/io.rs` now clears the buffer on `read_to_string`
failure instead of panicking. Non-UTF-8 files produce an empty string result.

---

## Arithmetic Safety

### ~~49. Integer arithmetic silently wraps on overflow; may collide with null sentinel~~ **FIXED (debug builds)**

**Fixed (T1-31):** All integer and long binary operators in `ops.rs` now use
`checked_add`/`sub`/`mul`/`div`/`rem` in debug builds and assert results do not
collide with the null sentinel. Bitwise ops get sentinel-only checks. Release builds
retain the fast unchecked path. The underlying sentinel design is unchanged — use
`long` for values near `i32::MAX`/`i32::MIN`.

---

## Store Safety

### ~~50. `addr_mut()` on a locked store returned a thread-local DUMMY buffer in release builds~~ **FIXED 2026-03-18 (T0-11)**

**Symptom (fixed):** In release builds (`#[cfg(not(debug_assertions))]`), `Store::addr_mut()`
returned a pointer into a 256-byte thread-local buffer when called on a locked store, silently
discarding any write instead of panicking. Debug builds already panicked. This meant that loft
code running in release mode could silently produce wrong results when a code path accidentally
wrote to a `const`-locked store.

**Fix:** Removed the DUMMY buffer entirely. `addr_mut()` now calls
`assert!(!self.locked, "Write to locked store at rec={rec} fld={fld}")` unconditionally in both
debug and release builds. The `Store::lock()` doc comment and the `unsafe impl Send` safety comment
were updated to reflect the new invariant.

**Tests:** `src/store.rs` (unit test `write_to_locked_store_panics`, `#[should_panic]`).

---

## Vector Self-Append

### ~~51. `v += v` (vector self-append) silently corrupted data~~ **FIXED 2026-03-18 (T0-12)**

**Symptom (fixed):** `Stores::vector_add()` read `o_rec` (the source vector's backing record
pointer) before calling `vector_append` / `vector_set_size`. Those two functions may reallocate
the backing store, making the old `o_rec` value stale. On self-append (`v += v`, where `db` and
`o_db` share the same backing record) the stale pointer was then used to copy bytes from freed
(or reused) memory, silently producing corrupt data.

**Fix:** `vector_add()` now detects self-append by comparing `db.store_nr == o_db.store_nr &&
dest_rec == o_rec` before any resize. When a self-append is detected, the source bytes are
snapshotted into a `Vec<u8>` first; the snapshot is written to the destination after resize.
For the same-store, non-self-append case, `o_rec` is re-read from the store after resize.

**Tests:** `vector_self_append_integers`, `vector_self_append_single` in `tests/issues.rs`.

---

## File I/O Error Surfacing

### ~~52. File I/O errors were silently discarded~~ **FIXED 2026-03-18 (T1-32)**

**Symptom (fixed):** `write_file()`, `read_file()`, and `seek_file()` in `src/state/io.rs`
used `unwrap_or_default()` / `unwrap_or(0)`, swallowing all OS-level I/O errors (permission
denied, disk full, invalid seek position) with no diagnostic output. Programs could not
distinguish a successful empty read from a failure.

**Fix:** The three error-swallowing call sites are replaced with `eprintln!` logging:
- `File::create` failure: logs path and OS error to stderr, then returns early.
- `f.write_all` failure: logs `"file write error: {e}"` to stderr.
- `f.read` failure: logs `"file read error: {e}"` and returns 0 bytes.
- `f.seek` failure: logs `"file seek error: {e}"` to stderr.

**Tests:** `file_write_error_does_not_panic` in `tests/issues.rs` verifies that a write to a
non-existent path does not panic and execution continues normally.

---

## See also
- [PLANNING.md](PLANNING.md) — Priority-ordered enhancement backlog
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — Language design inconsistencies and asymmetries
- [TESTING.md](TESTING.md) — Test framework, reproducing and debugging issues
- [../DEVELOPERS.md](../DEVELOPERS.md) — Debugging strategy and quality requirements for new features
