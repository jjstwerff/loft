
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
| ~~22~~ | `spacial<T>` diagnostic wording | — | **Done** — message now says "planned for 1.1+; until then use sorted<T> or index<T>" |
| 54 | `json_items` returns opaque `vector<text>` | Medium | **0.9.0:** first-class `JsonValue` enum (JObject / JArray / JString / JNumber / JBool / JNull); `json_parse` is the one entry point; old text-based surface withdrawn |
| 184 | `vector<i32>` ignores the `size(4)` annotation — elements stored + accessed as 8-byte i64 (same for `u32`, and symmetrically for `hash<i32>` / `sorted<i32>` / `index<i32>`) | Medium | **Workaround:** use `vector<integer>` and emit explicit `as i32` casts at binary-write sites (`f += val as i32`).  The type annotation looks narrower but storage + reads are both 8 bytes — trust the `integer` name and cast at boundaries. |
| P185 | Slot-aliasing SIGSEGV / heap corruption — a local declared AFTER an inner `body += <format-string>` accumulator, inside a `for _ in file(...).files()` loop (inline temporary, not a named var), gets assigned a slot that overlaps a still-live text buffer.  Teardown trips `OpFreeText` (op 118) on the aliased slot → SIGSEGV or `realloc(): invalid pointer`.  Discovered while debugging why `scripts/build-playground-examples.loft` corrupts its own output file mid-run. | **High** (safety: heap corruption) | **Workarounds** (both work): (a) declare the late local BEFORE the inner loop, or (b) hoist `file(...)` into a named variable (`d = file(...); for f in d.files()`).  Both nudge slot assignment away from the bad overlap. |
| ~~91~~ | Default-from-earlier-parameter | — | **Done** — call-site `Value::Var(arg_index)` substitution in the stored default tree; simpler than planned prologue approach |
| ~~135~~ | Sprite atlas row indexing swap | — | **Fixed** — canonical `(0,0) = screen-top-left`; canvas upload no longer pre-flips rows; OPENGL.md § Canvas coordinate convention.  Regression: 2×2 atlas corner check in `tests/scripts/snap_smoke.sh` / `make test-gl-golden` |
| ~~137~~ | `loft --html` Brick Buster runtime `unreachable` panic | — | **Fixed** — `Instant::now()` guard switched from `feature = "wasm"` to `target_arch = "wasm32"`; `host_time_now()` returns 0 on wasm32-without-wasm-feature; `n_ticks` gated identically. Tests: `tests/html_wasm.rs` (4 regression guards behind a serial mutex) |
| ~~139~~ | `_vector_N` slot-allocator TOS mismatch | — | **Fixed** — `gen_set_first_at_tos` emits `OpReserveFrame(gap)` when the allocator's slot is above TOS (zone-1 byte-sized vars left the gap). Tests: `tests/issues.rs::p139_*` |
| ~~136~~ | wrap-suite SIGSEGV on `79-null-early-exit.loft` | — | **Fixed** — `state/codegen.rs::gen_if` now resets `stack.position` to the pre-if value when the true branch diverges and `f_val == Null`; `is_divergent` recurses into `Insert`/`Block` wrappers (C56 `?? return` puts `Return` inside an `Insert` after scope analysis). Tests: `tests/wrap.rs::sigsegv_repro_79_alone` (un-`#[ignore]`d), `loft_suite` now covers the script. |
| ~~142~~ | `vector<T>` field panics when T is from imported file | — | **Fixed** — plain `use` now imports all pub definitions via `import_all` |
| ~~143~~ | SIGSEGV returning default struct from function iterating nested vectors | — | **Fixed** — `gen_set_first_ref_call_copy` (`src/state/codegen.rs`) now brackets `OpCopyRecord` with `n_set_store_lock(arg, true)` / `(arg, false)` for every ref-typed argument of the call.  `OpCopyRecord`'s existing `!locked` guard at `src/state/io.rs:1001` then skips the source-free when the source aliases one of the locked args (the P143 case: `return arg.field[i]` returns a DbRef into `arg`).  `src/scopes.rs::free_vars` was extended to free `__ref_*`/`__rref_*` work-refs at function exit so the non-aliased path's storage doesn't leak.  Tests: `tests/lib/p143_{types,entry,main}.loft` + `tests/issues.rs::p143_default_struct_return_from_nested_vector_use`. |
| ~~144~~ | Native codegen emits `*var_b` instead of `var_b` for `&` param forwarding | — | **Fixed** — `output_call_user_fn` detects `RefVar` → `RefVar` forwarding |
| ~~145~~ | SIGSEGV calling text-returning fn on multi-vector struct in cross-file package | — | **Fixed** — user function `n_to_json` collided with native stdlib `n_to_json` (JsonValue serializer) in `library_names`; codegen emitted `OpStaticCall` (native dispatch) instead of `OpCall` (user bytecode).  Fix: `generate_call` skips `library_names` lookup when the definition has a user body (`code != Value::Null`).  Tests: `tests/issues.rs::p145_text_return_multivec_struct_cross_file` |
| ~~146~~ | Wrap-suite store leak on user-fn alias return + deep-copy | — | **Fixed 2026-04-16** — added the missing companion to the existing skip-free branch in `src/scopes.rs::scan_set`: when the call's `has_ref_params == true` (codegen will take `gen_set_first_ref_call_copy`'s deep-copy path), strip the LHS variable's parser-inferred deps via `make_independent` so `get_free_vars` emits OpFreeRef at scope exit.  Tests: `tests/leak.rs::p146_script_95_alias_copy_leak`. |
| ~~147~~ | Wrap-suite leaks on scripts 62/76/81 (P146-family + harness quirk) | — | **Fixed 2026-04-16** — three subfixes: (a) `tests/wrap.rs::entry_point_names` now skips zero-param fns whose return type is non-Void (closes 62/76 — `rng_build_index()` / `svr_identity()` were being auto-invoked as entry points and their returned stores leaked; convention is that test entry points return Void); (b) `src/scopes.rs::scan_set` got a second companion for the `Set(v, Var(src))` deep-copy path (codegen takes `gen_set_first_ref_var_copy`) — strip v's deps so OpFreeRef is emitted (closes 81's I13 hidden `__iter_obj` deep-copy leak). |
| ~~148~~ | Wrap-suite leak on script 45 (A10 field iteration) | — | **Fixed 2026-04-16** — removed `clean_work_refs(work_checkpoint)` call from `src/parser/collections.rs::parse_field_iteration` (line 1761).  The unrolled loop creates 2 work-refs per field (FvFloat/etc + StructField); the `clean_work_refs` call marked ALL of them `skip_free`, preventing scope-exit cleanup of the orphaned work-refs from prior iterations.  With skip_free removed, `get_free_vars`'s `is_work_ref` check emits `OpFreeRef` for each.  Tests: `tests/leak.rs::p148_script_45_field_iteration_leak`. |
| ~~149~~ | SIGSEGV in `OpCopyRecord` for nested struct construction (script 76) | — | **Fixed 2026-04-16** — closed as a side-effect of the P147 `Set(v, Var(src))` dep-stripping fix.  The SEGV was caused by a `__ref_*` work-ref's store being freed prematurely (scope analysis treated it as borrowed due to non-empty dep), then `OpCopyRecord` reading from the freed store.  With deps stripped on the deep-copy path, stores are properly managed and the SEGV disappears.  Tests: `tests/leak.rs::p149_script_76_nested_struct_segv`. |
| ~~150~~ | Per-call leak: orphaned `__ref_*` placeholder when callee returns a fresh store | — | **Fixed 2026-04-16.**  When `m = user_fn(...)` with no visible Reference params, codegen pre-allocates a placeholder store via `OpConvRefFromNull` for the hidden `__ref_*` slot.  Both `src/scopes.rs:518-527` and `src/state/codegen.rs:1057-1064` marked `__ref_*` as `skip_free` to avoid double-free in the typical adoption case (callee writes into the placeholder, `m` and `__ref_*` alias).  When the callee instead returns a fresh store (early-return through a constructor call, or fall-through `T.parse(text)` — both shapes used by `lib/moros_map/src/moros_map.loft::map_from_json`), the placeholder was orphaned.  Severity was per-call (game loops at 60fps would saturate u16 store space in ~18 minutes).  Fix: drop both `set_skip_free` calls.  The runtime tolerates double-free as a no-op (`src/database/allocation.rs:103-105`'s `if store.free { return }`), so the typical adoption case is unaffected (both frees fire, second is no-op) and the orphan case now reclaims the placeholder via the existing `is_work_ref` check in `scopes.rs::get_free_vars`.  Tests: `tests/leak.rs::p150_*` (un-`#[ignore]`'d). |
| ~~151~~ | Forward-reference to struct-returning fn + field mutation corrupts variable type inference | — | **Fixed 2026-04-16.**  `parser/fields.rs::field()` silently dropped the field-name token when the receiver had unknown type in pass-1 (returning the original `Value::Var(x)` unchanged), so `x.v = 99` collapsed to `x = 99` for downstream assignment processing.  `parse_assign_op` → `change_var` then set x's type to the RHS expression's type (integer for `x.v = 99`).  Pass-2 rejected the now-resolved `x = callee()` returning the struct.  Fix: wrap `code` in `Value::Drop` when `field()` returns early on an unknown receiver — `code != Value::Var(x)` ⇒ `assign_var_nr` returns `u16::MAX` ⇒ `change_var` skips the spurious type update.  Tests: `tests/issues.rs::p151_forward_ref_struct_call_with_mutation`. |
| ~~152~~ | `s.vec_field = vec_var` silently dropped at runtime (data-loss); struct-field whole-replacement also breaks `&` mutation check | — | **Fixed 2026-04-16.** parse_assign_op now rewrites `field = vec_var` to OpClearVector + OpAppendVector (deep copy into the field) and `field = []` to OpClearVector alone; find_written_vars unifies first_arg_write so collection ops + OpCopyRecord count as writes through OpGetField destinations.  Tests: `tests/issues.rs::p152_*` (4 regression guards). |
| ~~153~~ | Local vector ≥187 elements transferred to a struct field via construction → element rewrites read back as null / corrupt the heap | — | **Fixed 2026-04-16.** Two adjacent bugs in `src/database/structures.rs`: `vector_set_size` wrote the new length to the pre-resize rec after `Store::resize` relocated the block; `vector_add`'s byte-copy branches used `new_db` captured before `vector_set_size` ran, so writes landed in freed memory.  Fix: track the relocation in `vector_set_size` (update `vec_rec` after resize); re-read the destination rec in `vector_add` after `vector_set_size`.  Also widened `parser/objects.rs::handle_field` to emit the deep-copy OpAppendVector for any non-Insert vector initializer (not just bare Var), fixing `C { v: build() }`.  Tests: `tests/issues.rs::p153_*` (4 guards). |
| ~~154~~ | `s.v = helper_fn(s.v, …)` wipes the field; `s.v = s.v` clears to empty | — | **Fixed 2026-04-16.** Introduced by the P152 lowering: `s.v = rhs` expanded to `OpClearVector(s.v); OpAppendVector(s.v, rhs, tp)` but RHS was evaluated AFTER the clear, so any helper reading `s.v` saw empty.  Fix: detect non-Var RHS in `parser/expressions.rs::parse_assign_op`, capture it to a fresh local temp BEFORE the clear, then append from the temp.  Self-identity `s.v = s.v` (IR-equal LHS/RHS) collapses to a no-op.  Tests: `tests/issues.rs::p154_*` (3 guards). |
| ~~155~~ | Push/mutate/undo/mid-assert/redo/final-read sequence SIGSEGVs in `OpGetVector` | — | **Fixed 2026-04-16.** `src/state/codegen.rs::generate_set` (reassignment path, lines 891-932) emitted `OpCopyRecord` with the 0x8000 "free source" flag around a user-fn call, but without the `n_set_store_lock` bracket that `gen_set_first_ref_call_copy` (first-assignment path) uses.  When the callee returned a DbRef aliased with a caller arg, the free-source flag freed the caller's arg store; later uses SIGSEGV'd in `OpGetVector`.  Fix: port the lock/unlock bracket onto the reassignment path (same pattern as P143).  Tests: `tests/issues.rs::p155_segv_undo_redo_midassert`. |
| ~~156~~ | `struct X { v: vector<C> }` where C is the name of a stdlib constant (e.g. `E`, `PI`) panics in `typedef.rs:309` instead of emitting the "struct conflicts with constant" diagnostic | — | **Fixed 2026-04-16.** `parser/definitions.rs::sub_type` now checks the element def's DefType before descending into the collection branch — emits a proper diagnostic for Constant / Function / Routine.  `typedef.rs::fill_database` soft-continues on an unresolved vector content type so undefined-element-type programs (`vector<Undef>`) also diagnose cleanly instead of panicking.  Tests: `tests/issues.rs::p156_vector_element_shadows_constant`. |
| ~~157~~ | Native codegen emits `*var_m` (dereference) instead of `var_m` for `&Struct` → `&Struct` forwarding when the call also triggers argument pre-evaluation | — | **Fixed 2026-04-16.** P144 added the RefVar → RefVar preserve-ref check to `generation/calls.rs::output_call_user_fn`, but the pre-eval re-emission in `generation/pre_eval.rs::output_code_with_subst` had its own arg-emitter that bypassed the check.  Any user-fn call whose args need pre-evaluation (nested field reads, side-effecting expressions) hit the bypassed path and failed `rustc` with `expected &mut DbRef, found DbRef`.  Fix: mirror the check into the pre-eval path.  Tests: `tests/issues.rs::p157_native_refvar_forwarding_with_preeval`. |
| ~~159~~ | `Type.parse(json)` fails on struct-enums with "Unknown field Type.parse"; only plain structs supported | — | **Fixed 2026-04-17.** Two fixes: (1) `parser/objects.rs::parse_var` extended `DefType::Enum` detection for `.parse(` with link/revert so `Enum.Variant` qualified syntax isn't broken.  (2) `database/format.rs` `ShowDb::write` wraps EnumValue in JSON mode as `{"VariantName":{fields}}` so the JSON walker's existing discriminant-tag handler (line 332: single-key Object → variant name) can round-trip.  Tests: `tests/issues.rs::p159_struct_enum_json_roundtrip`. |
| ~~161~~ | `for` loop over `&vector<Struct>` parameter → "Unknown type null" / "Unknown iterator type" | — | **Fixed 2026-04-17.** `parser/control.rs::for_type` and `parser/collections.rs::iterator` didn't unwrap `RefVar(Vector(...))` before matching, so the element type resolved to null and the iterator setup rejected the type.  Fix: add `RefVar` unwrap at the top of both functions.  Tests: `tests/issues.rs::p161_for_over_ref_vector`. |
| ~~160~~ | Vector element `v[i]` cannot be passed as `&` parameter — "assign to a variable first" | — | **Fixed 2026-04-17.** Two changes: (1) `parser/mod.rs` accepts "addressable" expressions (vector element, field access chains rooted in a Var) in `&` parameter positions via `is_addressable()` helper.  (2) `state/codegen.rs::generate_call` handles `OpCreateStack(non-Var expr)` by generating the expression first (pushes its DbRef onto the stack), then emitting OpCreateStack with the u16 offset pointing at the just-pushed result — previously `add_const` wrote nothing for `Type::Reference` args, leaving garbage in the code stream.  Tests: `tests/issues.rs::p160_*` (2 guards). |
| ~~158~~ | Trailing comma after last field in struct-enum variant triggers parse error | — | **Fixed 2026-04-17.** `parser/definitions.rs::parse_enum_values` (line 266) looped back on trailing comma instead of breaking.  Regular struct parsing (line 1380) already had `\|\| self.lexer.peek_token("}")` — ported the same guard.  Tests: `tests/issues.rs::p158_trailing_comma_enum_variant`. |
| ~~164~~ | Trailing comma after the LAST VARIANT of an enum declaration fails to parse | — | **Fixed 2026-04-17.** `parser/definitions.rs::parse_enum_values` — the outer variant-list loop's break-on-comma check mirrored P158's field-list guard: `if !self.lexer.has_token(",") \|\| self.lexer.peek_token("}") { break; }`.  Covers both plain enums and struct-enums.  Tests: `tests/issues.rs::p164_trailing_comma_enum_variant_list`, `p164_trailing_comma_plain_enum`. |
| ~~165~~ | Variable declaration `x: Enum = Variant { ... }` rejected as "cannot change type from `Enum` to `Variant`"; struct-enum variant treated as a distinct type from its parent enum | — | **Fixed 2026-04-17.** `src/variables/mod.rs::change_var_type` — added a subtype branch that accepts `(Type::Enum(parent_d, true, _), Type::Reference(rhs_d, _))` when `data.def(rhs_d).parent == parent_d`.  The struct-literal constructor for a struct-enum variant types the expression as `Reference(variant_d)`, not `Enum(...)`; the variant's `parent` field proves the subtype relationship with the annotated enum.  Tests: `tests/issues.rs::p165_enum_annotation_with_variant_rhs`. |
| ~~166~~ | `file().content()` on a non-UTF-8 binary file silently returned empty text — data-loss class: a real program reads a 300 KB `.glb` via `.content()` and sees `""` with no log, no warning, no diagnostic.  Surfaced by moros_render GLB export testing (`save_scene_glb` wrote a 305 KB file; `f.content()` returned `""`). | — | **Fixed 2026-04-17.** `src/state/io.rs::get_file_text` — `read_to_string` returns `Err(InvalidData)` on non-UTF-8 bytes; the prior `buf.clear()` branch was silent.  Fix: detect `ErrorKind::InvalidData` specifically and emit an actionable stderr warning naming the file path, the byte count, and the correct binary-read idiom (`f#format = LittleEndian; f#read(n)`).  Buffer still cleared for backwards-compat with callers that guarded on `""` as "could not read".  Companion loft-write skill update documents the binary idiom.  Tests: `tests/exit_codes.rs::p166_content_on_binary_file_warns`, `p166_content_on_text_file_no_warning`. |
| ~~167~~ | Trailing comma in a function-call argument list fails with `Too many parameters for n_<fn>`.  Third and final trailing-comma inconsistency after P158 (struct-enum variant field list) and P164 (enum variant list).  Repro: `rgb(10, 20, 30,)` or any multi-line call with a final comma. | — | **Fixed 2026-04-17.** `src/parser/control.rs::parse_call` — added `\|\| self.lexer.peek_token(")")` to the break-on-comma check in both the positional-arg loop (line 2770) and the named-arg loop (line 2718), mirroring the P158 / P164 pattern.  Tests: `tests/issues.rs::p167_trailing_comma_function_call_positional`, `p167_trailing_comma_function_call_multiline`. |
| ~~168~~ | `arguments()` leaks the full argv (binary path + CLI flags like `--interpret`) when zero script-level arguments are provided.  Surfaced by the 6a.18 moros_glb CLI tool. | — | **Fixed 2026-04-17.** `src/database/format.rs::os_arguments` used to fall through to `std::env::args_os()` when `user_args` was empty; P131's filter only ran through the `user_args` path.  Fix: always return `user_args` (an empty vector is a correct result) — removed the raw-argv fallback.  Tests: `tests/exit_codes.rs::p168_arguments_empty_when_no_script_args`. |
| ~~169~~ | Lambda-suggestion error message includes `-> <ret>` in the template, which misleads users to try `-> void` — but `-> void` fails with "Undefined type void" (loft has no `void` type; functions without `->` return void). | — | **Fixed 2026-04-17.** `src/parser/vectors.rs` — updated both lambda diagnostic messages ("Type annotations are not allowed in \|x\| lambdas" and "Cannot infer type for lambda parameter") to suggest `fn(x: <type>) { ... }` without the mandatory `-> <ret>`, and explicitly note that `-> void` is not a valid type.  Tests: `tests/exit_codes.rs::p169_lambda_suggestion_mentions_omitting_return_type`.  Underlying rule (loft has no `void` type for explicit annotation) kept as-is — the error-message quality fix is the user-facing deliverable. |
| ~~170~~ | `x = Struct {}; x = vec[i]; mutate(x)` — placeholder + vec-elem reassign + mutate trips the codegen assertion `Incorrect var x[N] versus M on n_<fn>`.  Surfaced while implementing stair dispatch in `build_hex_meshes`. | — | **Fixed 2026-04-17.** `src/parser/objects.rs::parse_object` — the in-place `v_set(x, Null) + OpDatabase(x, tp)` init branch required `is_independent(x) && type_matches`.  When a later `x = bs[i]` in the same function caused the parser's type-inferencer to tag x with a `dep` on bs, `is_independent(x)` returned false for the earlier `x = Bag {}` statement in second-pass — neither the if-branch nor the existing `else if !type_matches` fallback fired, so the struct-literal emitted only field-init calls that wrote into uninitialised storage.  codegen never saw a Set for x's first assignment, and later `generate_var(x)` asserted since x's slot sat above TOS.  Fix: extend the `else if` to also fire when `!is_independent && !first_pass` — routes the construction through a fresh work-ref (existing "new_object" path), which emits the required `v_set + OpDatabase` prelude and yields a `Block`-shaped RHS that the outer assignment copies through the normal Set path.  Tests: `tests/issues.rs::p170_struct_placeholder_then_vec_elem_reassign`, `p170_placeholder_conditional_then_reassign`.  A correct "Dead assignment — 'x' is overwritten before being read" warning now fires on the placeholder (expected behavior). |
| ~~171~~ | `--native` mode panicked at `types.rs:747` with `index out of bounds: the len is 124 but the index is 32859` during shutdown of any program that went through `OpCopyRecord` with the 0x8000 "free source" tag set.  Surfaced by running moros_render's `map_export_glb` → `map_build_scene` under `--native`. | — | **Fixed 2026-04-17.** `src/codegen_runtime.rs::OpCopyRecord` — native implementation was missing three pieces that the bytecode equivalent in `src/state/io.rs::copy_record` has: (1) the `raw_tp & 0x7FFF` mask before indexing `stores.types`, (2) `stores.remove_claims(&to, tp)` before the overwrite, (3) the `free_source` branch that releases the source store after deep copy.  Ported all three.  An agent audit of the rest of `codegen_runtime.rs` confirmed no other parity gaps.  Tests: `tests/exit_codes.rs::p171_native_copy_record_high_bit_does_not_panic` — native-mode run of `lib/moros_render/examples/isolated_stair.loft` exits 0 and writes a valid GLB.  **Note:** users running `--native` must rebuild the lib target (`cargo build --release --lib`) after touching loft sources; `cargo build --bin loft` alone doesn't refresh `target/release/libloft.rlib`. |
| ~~172~~ | `moros_map`'s chunk indexing used C-style truncating division (`q / 32`, `q % 32`), which produces *negative* `hx` and `cx` for negative `q`.  `map_set_hex(m, -1, 1, 0, h)` silently wrote to a wrong index, and `map_get_hex(m, -1, 1, 0)` read a default `Hex{}` back — data loss, no diagnostic.  Surfaced by walkable-editor SW/NW wall-mirror tests where the mirrored neighbour hex can have negative q. | — | **Fixed 2026-04-17.** `lib/moros_map/src/moros_map.loft` — added `chunk_idx_32(v)` / `hex_idx_32(v)` helpers that implement Euclidean division (`chunk = floor(v/32)`, `hex ∈ [0, 31]`).  Replaced every `q / 32`, `r / 32`, `q % 32`, `r % 32` in `map_has_chunk`, `map_ensure_chunk`, `map_set_hex`, and `map_get_hex`.  Tests: `lib/moros_map/tests/negative_coords.loft` — read/write round-trips through negative q and negative r survive the chunk dispatch. |
| ~~173~~ | Intra-package `use` cycle (e.g. `player.loft: use collide; collide.loft: use player;`) failed with downstream "Undefined type X" because `apply_pending_imports` ran `import_all` before the partner file's definitions were registered.  Discovered while wiring moros_sim's `resolve_move`. | — | **Fixed 2026-04-17.**  `src/parser/mod.rs` now defers `DefType::Unknown` diagnostics via `typedef::actual_types_deferred` into `Parser.deferred_unknown`, and after each pass calls `resolve_deferred_unknowns` to (1) re-apply every `applied_imports` entry via `Data::import_all_overwrite` / `import_name_overwrite` (overwriting any target-source Unknown stub with the now-registered real def), then (2) resolve each deferred stub by one of: (a) the stub got upgraded in-place by a later `parse_struct` — rewrite its own `Type::Unknown(stub)` refs via `rewrite_unknown_refs(stub, stub)`; (b) `source_nr` now points to a different real def — rewrite refs to that def; (c) still unresolved — emit the original "Undefined type" at the stored position.  New helpers: `Data::import_all_overwrite`, `Data::import_name_overwrite`, `Data::rewrite_unknown_refs`, `typedef::actual_types_deferred`.  Tests: `tests/imports.rs::p173_intra_cycle_resolves_cross_file_types` — two files that `use` each other resolve their cross-file types; `tests/data_import.rs` (8 unit tests) exercise the new Data helpers directly. |
| ~~174~~ | Reported as "`match` on struct-enum with field capture fails to parse".  False alarm — the `Expect token ,` diagnostic was simply flagging missing commas between match arms (the match parser requires `,` or `}` after each arm body; the original attempted code had neither).  Match with struct-enum field capture via `Variant { field } => { ... },` works correctly today.  Verified with a one-off fixture that parsed + executed green. | — | Withdrawn 2026-04-17.  No code change.  User-facing takeaway: add commas between arms.  The existing "Expect token ," diagnostic does in fact name the missing separator; just not the arm boundary.  Could be improved by clarifying "expected ',' between match arms" but that's a diagnostic polish rather than a bug. |
| ~~175~~ | File-scope `pub NAME: vector<text> = [literal1, literal2, ...]` declared the constant but didn't populate it — `len(NAME)` at runtime returned 0.  `vector<integer>` / `vector<float>` / `vector<single>` / `vector<long>` worked correctly.  Surfaced while adding `HEIGHT_STEP_LABELS` / `WALL_PALETTE_NAMES` to `moros_ui::panel`. | — | **Fixed 2026-04-17.** `src/compile.rs::extract_literal_values` — added `OpSetText` to the recognised per-element set-op list and `Value::Text(_)` to the matched literal variants.  `src/compile.rs::build_const_vectors` — added a `Value::Text(v)` arm that mirrors the runtime `OpSetText` path (`store.set_str(v)` to allocate the string in the CONST_STORE, then `set_int` to write the returned record number into the text field).  Tests: `tests/issues.rs::p175_vector_of_text_constant_populates` — both `vector<integer>` and `vector<text>` constants round-trip through `len()` / element indexing. |
| ~~176~~ | `&T` parameter passed to a method-style callee (`self: T` signature) that mutates via `self.field += …` tripped the "Parameter '<p>' has `&` but is never modified; remove the `&`" compile error.  The compiler didn't see the callee's internal `self.field` mutation as modifying the caller's `&T` argument.  Minimal repro: `fn add(self: Box, x: integer) { self.items += [x]; } fn caller(b: &Box) { add(b, 1); }` → error at caller's `b`. | — | **Fixed 2026-04-18.**  `src/parser/mod.rs::find_written_vars` now threads a memoised `callee_param_writes(fn_nr, data, cache)` helper that walks each called user-fn's body once to compute which of its params are written (directly or transitively), then marks the matching caller args as written via `collect_vars_in` (so wrapped sources like `OpCreateStack(Var(wv))` from the P179 path still propagate).  Placeholder entries break recursion cycles before descending; natives (`def.code == Value::Null`) are skipped since their effects are already expressed by the existing OpSet* / OpAppend* / OpCopyRecord syntactic patterns.  The analysis is additive (never retracts a prior mark), so the "really read-only" diagnostic continues to fire on parameters that genuinely aren't touched.  Tests: `tests/issues.rs::p176_ref_param_method_style_mutation` (canonical repro), `p176_transitive_forwarding_three_levels` (fixpoint / monotone merge), `p176_recursive_self_call_terminates` (cycle guard).  `lib/moros_ui/src/editor_click.loft` refactored to use the natural `route_click(p, st.es_tools, mx, my)` call (the inlined tool-selection workaround removed).  Full workspace suite green (1194 passes). |
| ~~177~~ | `is`-capture on a struct-enum variant read **garbage** for the captured integer in `moros_ui::panel.loft::route_click`'s pre-workaround form.  Observed `tb_id = 24507` while the enum printed as `UhToolButton {tb_id:2}` just above.  Same root cause as P178 (slot-allocator orphan-placer collides with argument slots); the `24507` value is consistent with reading the bytes of a still-live arg slot after the capture write landed in the wrong place. | — | **Fixed 2026-04-17** (via the P178 fix).  Applying the moros_ui round-trip test on top of the P178 fix confirms the original struct-enum shape with embedded fields now captures correctly.  P177 stays in history as the user-visible reproducer. |
| ~~181~~ | An inline struct-returning call inside a format-string interpolation — `"got {map_get_hex(e.es_map, q, r, cy).field}"` — was transformed by scope analysis's inline-lift pattern into `__lift_N = map_get_hex(...)` + `OpCopyRecord(src, to=__lift_N, tp = def_nr \| 0x8000)`.  The `0x8000` free-source flag freed the callee's return source after copying.  When the callee returned a view (e.g. `gh_c.ck_hexes[idx]`), the freed "source" was part of the owning vector's store — corrupting sibling fields. | — | **Fixed 2026-04-18** across two phases.  **Phase 1**: `src/state/codegen.rs` now clears the `0x8000` flag at both OpCopyRecord emission sites (`gen_set_first_ref_call_copy` and the reassignment path in `generate_set`) when the callee's `returned.depend()` is non-empty.  Covers consistent-view callees (e.g. `fn first_inner(c: Container) -> Inner { c.items[0] }` inferred as `Inner["c"]`).  **Phase 1b**: `src/parser/control.rs::parse_return` now mirrors `block_result`'s Reference / Enum(struct-enum) arms so mid-body `return expr;` statements merge the return expression's deps into `def.returned` (`text_return` was already symmetric; `ref_return` was not).  After Phase 1b, `fn first_or_empty(c, idx) -> Inner { if ... return c.items[idx]; Inner{} }` infers `-> Inner["c"]`, the gate fires, 0x8000 is cleared, and the corruption stops.  `lib/moros_sim/tests/picking.loft::test_edit_at_hex_raise` now uses the natural inline form without the hoist workaround.  **Known trade-off**: for the owned-fallback branch (e.g. `Inner {}`) the fresh store is no longer freed by 0x8000 → a small per-call leak on the fallback branch only.  Acceptable — corruption is the worst bug class, the fallback is typically an error path, and P120 regressions (8 tests, tail-only struct returns) stay green.  Vector-return arm intentionally deferred (promoting globals/locals broke moros_ui).  See `doc/claude/plans/finished/00-inline-lift-safety/01b-return-dep-inference.md`.  **Tests**: `tests/lib/p181_inline_field_access.loft` + `tests/issues.rs::p181_inline_field_access_format_string`; 16 snippet variants in `doc/claude/plans/finished/00-inline-lift-safety/snippets/` including `07_mixed_return.loft` and `17_println_two_calls.loft`. |
| ~~180~~ | Assigning a `single` (f32) literal to a `float` (f64) struct field was silently accepted by the parser and corrupted the record at runtime (interpreter panicked `index out of bounds` in the allocation layer; native codegen leaked raw rustc E0308 to the user).  Surfaced while writing Step 16 mouse-look tests. | — | **Fixed 2026-04-18.**  Root cause: the constructor path (`src/parser/objects.rs::handle_field:1399`) and return-type path (`src/parser/control.rs::parse_return:2438`) already route their RHS through `self.convert()`, which wraps with the registered `OpConvFloatFromSingle(single) -> float` op for widening and rejects narrowing with a diagnostic.  The post-construction assignment path at `src/parser/expressions.rs` skipped that funnel — it had only a hand-rolled `OpConvLongFromInt` special case for `integer → long` and otherwise emitted `OpSetFloat` blindly with an f32 payload.  Fix replaces the special case with a general `convert()` call guarded by: `op == "="` (compound assignments stay on their `compute_op_code` path); the TARGET type is a scalar (integer/long/float/single/boolean/character/text — collection targets like `vector`/`hash`/`sorted` have dedicated handling in `towards_set`); neither side is `Unknown` (bounded-generic templates carry placeholder types until monomorphisation); the source is not `Null` (null-assignment has special remove-from-collection semantics downstream).  The new `convert()` picks up every registered widening (`integer → long`, `integer → float`, `single → float`, …) and produces "Cannot assign {T} to a field of type {U} — use 'as {U}' to cast explicitly" on unrelated / narrowing mismatches.  Policy is now uniform across constructor, return, and post-construction assignment.  Tests: `tests/issues.rs::p180_single_literal_into_float_field` (un-gated from `#[ignore]`), `p180_int_widens_to_long_field` (guards the former hand-rolled path against regression).  `tests/lib/p180_single_to_float_field.loft` fixture kept as documentation.  Full workspace suite green (1196 passes, 0 failures). |
| ~~179~~ | Passing a `&struct.field` expression as an argument alongside other arguments produced silently corrupted reads in the callee.  Given `fn f(n: integer, r: &T) { r.x = n; }` and `o = Outer { ... }`, the call `f(42, o.field)` set `r.x = 3` (not 42) — `n` inside the callee read a fragment of the DbRef representation of the `&` arg.  Boolean / text / second `&` args corrupted similarly.  Two-`&`-siblings variant (`f(o.a, o.b)`) lost BOTH mutations. | — | **Fixed 2026-04-17.**  Root cause: `src/parser/mod.rs::convert()` wrapped every non-Text `&T` source in `OpCreateStack(code)` regardless of source shape.  For non-Var sources the codegen shortcut at `src/state/codegen.rs:1545-1552` pushed the source DbRef AND a second ref DbRef — 24B for a 12B `&` arg — so `args_base = stack_pos - args_size` landed in the middle of the source and the preceding by-value arg read the tail of it.  Fix routes non-Var sources through a `work_refs` local: emits `Value::Insert([Set(wv, expr), OpCreateStack(Var(wv))])` which `src/scopes.rs::scan_args` hoists into the enclosing statement list (Insert doesn't form a scope, so the work-ref lives at function scope — its slot survives the call).  The work-ref is marked `skip_free` because it holds a borrowed DbRef to an existing store; without that, scope-exit `OpFreeRef` would decrement ref_count to 0 after one call and dangle the caller's owning reference across a loop.  Tests: `tests/lib/p179_ref_field_arg_corrupts_siblings.loft` (6 previously-failing + 1 control case now 7/7); `tests/issues.rs::p179_ref_field_arg_corrupts_sibling` (un-gated from `#[ignore]`).  Native path required no changes — scope-analysis hoisting produces the same `Set(wv, _)` + `Call(OpCreateStack, [Var(wv)])` shape that `src/generation/pre_eval.rs::create_stack_var`'s existing direct-Var branch already handles. |
| ~~178~~ | Slot-aliasing bug in the zero-scope path of the slot allocator.  When a function body's IR root is `Value::Insert` (multi-statement body with a trailing expression), `assign_slots`'s `process_scope` returns early at the `_ => return` arm without placing any locals.  All locals fall through to `place_orphaned_vars`, which started candidate slots at `0` — i.e., at the frame base.  Arguments have `stack_pos == u16::MAX` during `assign_slots` (codegen sets their positions later), so the per-var conflict check at line 413 skips them (`jv.stack_pos == u16::MAX → return false`), and orphans happily claim slot 0 / 4 / 12 / ..., overlapping the args at runtime.<br><br>Repro at `tests/lib/p178_slot_alias.loft`: a function `fn router(dummy: integer, tools: &FakeTools) -> Ui { rc = hit_test(); if rc is Variant { tb_id } { tools.ft_cur = tb_id; } rc }` captured `tb_id` as 0 instead of 2.  Manifestations across shapes: silent `0` (this repro), "index out of bounds" panic (shape with extra struct-by-value param), or `24507`-style garbage integer (P177's moros_ui route_click). | — | **Fixed 2026-04-17.** `src/variables/slots.rs::place_orphaned_vars` — added `local_start` parameter and initialised `candidate = local_start` instead of `0`.  Orphan locals can no longer overlap the argument + return-address region.  `assign_slots` threads `local_start` into the call.  The `process_scope` `_ => return` early-exit on non-Block IR (the behaviour that makes ALL locals orphans in the first place) is preserved — the fix targets the consequence, not the cause.  Tests: `tests/issues.rs::p178_is_capture_slot_alias` (now passing, previously `#[ignore]`-gated); `tests/lib/p178_slot_alias.loft` fixture.  Full workspace `cargo test --release` green (no regressions). |

---

## Interpreter Robustness

### ~~86~~. Lambda capture — FULLY RESOLVED (closures shipped)

With real closure capture in 0.8.3, the original codegen error
`[generate_set] ... Var(1) self-reference — storage not yet allocated`
is no longer reachable.  The parser-level mitigation
(*"lambda captures variable X — closure capture is not yet supported"*)
is also gone since the feature is implemented.

The original reproducer now runs correctly end-to-end:

```loft
fn test() {
    count = 0;
    f = fn(x: integer) { count += x; };
    f(10); f(32);
    assert(count == 42);   // passes
}
```

**Regression guards:**
- `tests/issues.rs::p1_1_lambda_void_body` — runtime behaviour (`count == 42`)
- `tests/parse_errors.rs::capture_detected` — parse succeeds, no diagnostic
- `tests/parse_errors.rs::no_capture_no_error` — no false capture positives
- `tests/parse_errors.rs::local_not_captured` — lambda-local vars don't trigger capture

No open action.  Kept here as a marker for CHANGELOG readers; remove on
the next 0.9.0 maintenance sweep.

---

### ~~91~~. Default-from-earlier-parameter — DONE

**Symptom:** `fn make_rect(w: integer, h: integer = w)` fails with
*"Unknown variable 'w'"*; the default expression cannot reference
earlier parameters of the same function.

**Semantics decision:** the default is evaluated *at function entry*,
not at the call site.  That is deliberately different from struct-
field `init(expr)`, which evaluates once at construction.  Required
because the default's whole point is to see the earlier parameters'
call-site values.

**Fix path (three parts):**
1. `parse_arguments` — accept `= expr` referencing earlier params.
   Earlier params are injected into `self.vars` as arguments
   (via `add_variable` + `become_argument` + `defined`) before
   parsing the default, then removed before returning so the
   caller's own argument-registration is unaffected.
2. Call site — pass a supplied-args bitmap (one bit per argument
   with a default) so the callee knows which defaults to evaluate.
3. Function prologue — emit `if !supplied(N) { arg_N = <default> }`
   for each defaulted parameter, using the bitmap bit.

**Scope: M**, three moving parts.  The first naive attempt hit
two-pass state issues in the parser alone; call-site + prologue are
still to do.

---

## Web Services

### 60. Hash iteration — designed 2026-04-13

Full design in CAVEATS.md C60.  Summary: `for e in hash { … }`
iterates in ascending key order, loop variable is the record (no
tuple destructuring).  Implementation is a pre-loop lift that walks
all records of the struct type into a scratch `vector<reference<T>>`,
sorts by extracting key fields, and iterates the sorted vector.
Inefficient by design (O(n log n) per loop); determinism beats
unspecified-order for a scripting language.

Scope: parser routing at `src/parser/fields.rs:599`, a new
`parse_iter_hash` in `src/parser/collections.rs`, a record-walk
helper in `src/database/search.rs` (or reuse the `validate` walk at
line 327), and one new opcode (`OpHashCollect` or `OpHashIterSetup`).

Scope honestly M–MH.  Two days of focused work; the design is
concrete and the scope is bounded.

---

### 54. `json_items` returns opaque `vector<text>` — 0.9.0

**Symptom:** `json_items(body)` returns `vector<text>` where each
element is either a JSON object body or garbage.  The caller writes
`MyStruct.parse(body)` and gets a partial zero-value struct on
malformed input — no type checking, no diagnostic.

**Decision:** replace the text-based JSON surface with a first-class
`JsonValue` enum.  No newtype-around-text half-measure — the newtype
would keep the text surface, its shape predicates would be runtime
peeks into the string, and `.parse` would still run a separate parser
over every element.  Doing the parse once into a typed tree and then
indexing / matching that tree is simpler, faster, and covers the
dynamic-shape use case too.

```loft
pub enum JsonValue {
    JNull,
    JBool   { value: boolean },
    JNumber { value: float not null },   // IEEE-754 per RFC 8259
    JString { value: text },
    JArray  { items:  vector<JsonValue> },
    JObject { fields: vector<JsonField> }
}

pub struct JsonField { name: text, value: JsonValue }

// Parse + diagnostics
pub fn json_parse(raw: text)               -> JsonValue;
pub fn json_errors()                       -> text;     // RFC 6901 path + line:col

// Read surface
pub fn kind(self: JsonValue)               -> text;     // "JNull" .. "JObject"
pub fn len(self: JsonValue)                -> integer;  // null on non-container
pub fn field(self: JsonValue, name: text)  -> JsonValue; // JObject only; JNull on miss / wrong kind
pub fn item(self: JsonValue, index: integer) -> JsonValue; // JArray only; JNull on OOB / wrong kind
pub fn has_field(self: JsonValue, name: text) -> boolean;
pub fn keys(self: JsonValue)               -> vector<text>;
pub fn fields(self: JsonValue)             -> vector<JsonField>; // values deep-copy

// Typed extractors — null on kind mismatch
pub fn as_text(self:   JsonValue) -> text;
pub fn as_number(self: JsonValue) -> float;
pub fn as_long(self:   JsonValue) -> long;
pub fn as_bool(self:   JsonValue) -> boolean;

// Write surface
pub fn to_json(self: JsonValue)            -> text;     // canonical RFC 8259
pub fn to_json_pretty(self: JsonValue)     -> text;     // 2-space indent for non-empty containers

// Construction helpers
pub fn json_null()                                 -> JsonValue;
pub fn json_bool(v: boolean)                       -> JsonValue;
pub fn json_number(v: float)                       -> JsonValue;  // non-finite → JNull
pub fn json_string(v: text)                        -> JsonValue;
pub fn json_array(items: vector<JsonValue>)        -> JsonValue;  // deep-copies items
pub fn json_object(fields: vector<JsonField>)      -> JsonValue;  // deep-copies fields

// Schema-driven (P54 step 5 — pending)
pub fn parse(self: Type, v: JsonValue) -> Type;   // `MyStruct.parse(v)`
```

`JObject.fields` is stored as `vector<JsonField>` rather than the
originally-designed `hash<JsonField[name]>` — the hash form is a
0.9.0 follow-up once hash iteration and nested struct-enum-in-hash
layouts are exercised end-to-end.  Linear scan is fine for the
object sizes typical in configuration / API responses.

The old `json_items` / `json_nested` / `json_long` / `json_float` /
`json_bool` surface documented in [PLANNING.md](PLANNING.md) § H2
is withdrawn.  All JSON work routes through `json_parse` →
`JsonValue` from 0.9.0 onward.

Full landing plan in [QUALITY.md § P54](QUALITY.md#active-sprint--p54-jsonvalue-enum).

---

## Graphics / WebGL

### ~~135~~. Sprite atlas row indexing swap — FIXED

Canvas upload no longer pre-flips rows; `TEX_VERT_2D` samples with
identity V.  Canvas-top = GL TC.y = 0 on all three backends (native
OpenGL, WebGL/wasm, `--html` export), and `lib/graphics/native/src/lib.rs`
+ `lib/graphics/js/loft-gl.js` + `doc/loft-gl-wasm.js` now agree on the
same orientation.  Canonical convention locked in
[OPENGL.md § Canvas coordinate convention](OPENGL.md).

Regression guard: 2×2 atlas corner check added to
`tests/scripts/snap_smoke.sh` — asserts sprite 0/1/2/3 render
red/green/blue/white (matching the atlas's top-row / bottom-row
layout).  `make test-gl-golden` fails if any future upload / shader /
projection change reintroduces a row swap.

Original issue kept below for context.

### 135 (historical). Sprite atlas row indexing swap

**Severity:** Low — cosmetic.

**Symptom:** in a 2×2 sprite atlas, sprites 1 and 3 appear at
swapped canvas positions when drawn via `draw_sprite`.  The smoke
test (`tests/scripts/snap_smoke.sh`) pixel-samples the affected
corners and confirms the mis-placement is reproducible.

**Root cause:** interaction between `gl_upload_canvas`'s Y-flip
(row reversal during upload, `lib.rs:837`), `draw_sprite`'s
V-coordinate computation (`graphics.loft:773-776`), and the
orthographic projection in `create_painter_2d` (`-2/H`, which also
flips Y).  Two of the three flips cancel; the third lands in an
unexpected quadrant, so row indexing into the atlas is off by one
row.

**Workaround:** arrange sprites in a single row (N×1 atlas) until
the flip sequence is normalised.

**Fix path:** decide a single canonical Y direction (screen-origin
top-left) and remove the compensating flip from one of the three
sites — most naturally the upload, since it's the one introduced
last.  Test: extend `snap_smoke.sh` to assert all four corners of
a 2×2 atlas are placed correctly.

---

### ~~137~~. `loft --html` runtime `unreachable` panic — FIXED

Root cause: `Stores::new()` called `std::time::Instant::now()` on the
`--html` build (wasm32-unknown-unknown without the `wasm` feature).
`Instant::now()` panics on this target with no time source; the panic
compiles to `(unreachable)` in release builds, producing the infamous
trap on the very first `loft_start` call — before any user code or
host import ran.

Fix: switch the start-time guard from `#[cfg(feature = "wasm")]` to
`#[cfg(target_arch = "wasm32")]`.  Any wasm32 target uses the
`start_time_ms: i64` field; feature-gated path calls the host bridge,
no-feature path uses 0 as a benign epoch stub.  `n_ticks` on wasm32
without the feature returns 0 (no time bridge, same contract).

Verified: `fn main() { println("hello"); }` compiled with
`loft --html` and instantiated in Node with a `loft_host_print` stub
prints "hello from loft" cleanly.

Test strategy used to find it: debug-built WASM carries Rust panic
string symbols in the stack trace — `noop_debug.wasm` stack showed
`std::time::Instant::now → loft::database::Stores::new` as the panic
origin.  Release builds strip the names and reduce the trap to a bare
`unreachable`, which is why previous diagnostic attempts bottomed out
at "panic in bytecode dispatch, not a host call".

### 137 (historical). `loft --html` Brick Buster: runtime `unreachable` panic

**Severity:** Medium — breaks the deployed `brick-buster.html` on
GitHub Pages; the wasm instantiates but panics as soon as `loft_start`
runs.

**Symptom:** the browser reports

```
Uncaught (in promise) RuntimeError: unreachable executed
    at wasm-function[234]:…
    at wasm-function[229]:…
    …
    at wasm-function[258]:…
```

Reproducible in Node with stub imports: `loft_start()` throws
`unreachable` on the first call, regardless of whether asyncify is
enabled (tested with `wasm-opt -O1 --asyncify` and with no asyncify
pass at all).

**Narrowed down:**

- Not an instantiation failure — all 25 host imports (`loft_gl.*`,
  `loft_io.*`) are present and the wasm compiles.  Pull request #168
  fixed the earlier instantiation-time bug by switching `-Oz` to
  `-O1`; this new failure is at *runtime*, not at instantiate.
- Not a generated-Rust `todo!()` — `grep -c 'todo!'` on the emitted
  `/tmp/loft_html.rs` returns 0.  Every `#native` function has a real
  extern declaration + call.
- Not an asyncify artefact — reproduces with `wasm-opt -O1
  --strip-debug --strip-producers` (no `--asyncify`).
- The panic originates in generated bytecode dispatch, not in a
  host-call — the call stack has no import frames.

**Workaround:** native mode (`make play`) runs the game correctly;
only the browser build is broken.

**Fix path:**

1. Capture the pre-wasm-opt `/tmp/loft_html.wasm` and instantiate it
   directly in Node to confirm the panic is in the rustc output, not
   a wasm-opt transformation.
2. Bisect which `#native` function's return path is unsafe: stub
   each import individually with a `throw new Error(name)` sentinel
   and see which one is hit last before the unreachable — that
   narrows the loft function whose emitted Rust body diverges.
3. Inspect the emitted Rust for that function in
   `src/generation/dispatch.rs::output_native_direct_call` — likely
   a type-marshalling mismatch between the loft signature and the
   generated `extern "C"` prototype (e.g. a `text` param that
   should pass `ptr, len` but was emitted as a single `i32`).
4. Add a browser-path assertion to `make game` that instantiates
   the built wasm in Node and runs `loft_start` against `loft-gl-wasm.js`
   stubs, failing CI if it panics.

**Tracking:** discovered 2026-04-12 while verifying the
`make play` target.  Native path works; browser path wedged.

---

### 138. `--native` rustc E0460: `rand_core` version mismatch

**Severity:** Medium — blocks `loft --native <script>` and `make play`
on a checkout where `cargo build --release --bin loft` has run without
`--lib`.

**Symptom:** `rustc` fails compiling the generated `/tmp/loft_native.rs`
with

```
error[E0460]: found possibly newer version of crate `rand_core` which `loft` depends on
  --> /tmp/loft_native.rs:16:1
   |
16 | extern crate loft;
   | ^^^^^^^^^^^^^^^^^^
   = note: the following crate versions were found:
           crate `rand_core`: …/librand_core-<hashA>.rmeta
           crate `rand_core`: …/librand_core-<hashB>.rmeta
           crate `rand_core`: …/librand_core-<hashC>.rmeta
           crate `loft`: …/libloft.rlib
```

The E0460 cascades: every subsequent `use loft::codegen_runtime::*;`
fails to resolve, producing 700+ "cannot find function `OpNewRecord`"
/ `cr_call_push` / `OpFreeRef` / `n_set_store_lock` etc. E0425 errors.
The generated source itself is fine — rustc can't load the `loft` crate.

**Root cause:** cargo's incremental-build state has `libloft.rlib`
referencing an older `rand_core` rmeta hash than what's currently in
`target/release/deps/`.  This happens when `--bin loft` rebuilds but
`--lib` is left stale.

**Workaround (already shipped):** `make play` step 1 now runs
`cargo build --release -q --lib --bin loft` so the rlib is always
current.  A manual `cargo clean && cargo build --release` is the
fallback when a user's tree has other stale artefacts.

**Mitigation (shipped, `src/main.rs`):** the `--native` driver now
captures rustc's stderr and, on E0460 with "rand_core" or
"possibly newer version of crate", prints an actionable hint —

```
loft: native compilation failed because the cached `libloft.rlib`
references a different dependency version than the one now in
`target/release/deps/`.

Fix:  cargo build --release --lib --bin loft
Or:   cargo clean && cargo build --release
```

This replaces the previous 700-error cascade with a single recovery
instruction.  Test: introduce a stale rlib (`cargo build --bin loft`
after modifying a dependency version) and run
`loft --native <any-file>` — the hint should appear.

---

### ~~139~~. `_vector_N` slot-allocator TOS mismatch — FIXED

**Fix:** `src/state/codegen.rs::gen_set_first_at_tos` now handles
`pos > TOS` by emitting `OpReserveFrame(pos - TOS)` and advancing
codegen's TOS to match.  The runtime stack pointer moves through
the zone-1 byte-sized variable's slot (plain enum or boolean, already
written via `OpPutEnum` / `OpPutBool`), so the subsequent init
opcode writes to the correct zone-2 slot.

**Root cause** (confirmed by trace):
- Slot allocator places byte-sized zone-1 vars (1-byte plain enum,
  1-byte boolean) at fixed slots just below the zone-2 frontier.
- Codegen's TOS counter advances by the op deltas of the per-statement
  push/pop cycle.  `OpConstEnum` pushes 1, `OpPutEnum` pops 1, net
  zero.  The 1-byte zone-1 slot stays "written but not counted in TOS".
- When the next zone-2 `Set(v, …)` runs, slot = zone2_start but TOS =
  zone2_start - 1.  The former `pos == TOS` assert fired.
- Reproducer: plain enum + vector + same-type loop write
  (5 lines — see `tests/issues.rs::p139_enum_vec_same_type_write_through_loop`).

**Why `stack.position = pos` alone failed** (the earlier naive
attempt): the runtime stack pointer wasn't bumped, so subsequent
reads pulled from the zone-1 slot as if it were the zone-2 slot.
`OpReserveFrame` bumps the runtime pointer to match the codegen
pointer.

**Tests:** `tests/issues.rs::p139_enum_vec_same_type_write_through_loop`,
`p139_enum_vec_two_loops_same_function`,
`p139_bool_vec_write_through_loop`.  `tests/wrap::enums` (pre-existing
snapshot test that originally surfaced the bug) stays green.

---

### 139.  *(historical note — see entry above for the fix)*

Discovered 2026-04-12 during C61.local unconditional-reject attempt;
narrowed 2026-04-12 to a 5-line reproducer; fixed 2026-04-13 via
instrumented trace + `OpReserveFrame` in the set-first path.

**Symptom:** codegen panics from `src/state/codegen.rs:922`:

```
[gen_set_first_at_tos] '_vector_3' in 'n_main': slot=N but TOS=N-1
— caller must ensure TOS matches the variable's slot before calling
```

**Minimal reproducer** (plain enum + vector + same-typed
cross-variable assignment inside a for-loop body):

```loft
enum Dir { North, East, South, West }
fn main() {
    dirs = [North, East, South, West];
    first_d = North;
    for elem in dirs { first_d = elem; }
}
```

Trips `slot=N but TOS=N-1` — slot > TOS by exactly 1 byte, matching
the enum discriminant size.  An alignment gap the allocator reserved
(for the vector temp `_vector_N`) that the TOS counter didn't advance
through.

**Not a simple "advance TOS" fix:** naïvely setting
`stack.position = pos` in `gen_set_first_at_tos` (the mirror of the
existing `pos < TOS` correction) makes the assert pass but produces
garbage at runtime (`index out of bounds: the len is 4 but the
index is 768`).  The padded byte isn't actually free — it's either
initialised by a prior op the allocator expected to run or the
slot was pre-assigned without accounting for the enum's 1-byte
discriminant.

**Real fix path:** phase-B dump at the `_vector_N` creation site —
what op produces the slot offset?  what writes into the alignment
gap?  The assert is only the symptom; the root is in either
`src/variables/slots.rs` (slot pre-assignment not accounting for
byte-sized discriminants) or one of the `OpNewVector*` emit sites.

**Why it matters now:** blocks C61.local's stdlib rename sweep.
Latent in main today — no CI exercises the triggering layout — but
independently reproducible via the enum + for-loop snippet above.

**Discovered:** 2026-04-12 during C61.local unconditional-reject
attempt (commit b716d1d, reverted).  Narrowed 2026-04-12 via a
5-line reproducer and a failed naïve fix.

---

## 136. Wrap-suite SIGSEGV on `79-null-early-exit.loft` — FIXED

**Root cause.** `state/codegen.rs::gen_if` (the `f_val == Value::Null`
branch) left `stack.position` at the true-branch's end-state after emitting
a divergent true branch.  At runtime the join point is reached only via
the `OpGotoFalseWord` jump, where `stack_pos` equals the pre-if value —
so every subsequent `Var*` / `Put*` op encoded `var_pos = codegen_stack −
slot` was 4 bytes off.  Writes through `_ncr_1` / `val` corrupted the
return-address slot; after a handful of `safe_double` calls the
interpreter read a small bytecode offset as a return address and
re-entered already-returned code, growing the stack by ~12 bytes per
iteration until it overflowed the 8008-byte stack store.

`is_divergent` also did not recognise `Value::Insert([..., Return(...)])`
— the shape `scopes.rs` produces when it wraps a `Return` with
`free_vars` cleanup.  So even the else-present branch's divergence reset
(line 520-524) silently missed the C56 case.

**Fix.** Two small edits in `src/state/codegen.rs`:
- Widen `is_divergent` to recurse into the last op of `Value::Insert` and
  `Value::Block`.
- In the `*f_val == Value::Null` arm of `gen_if`, reset
  `stack.position = stack_pos` when the true branch is divergent.

**Tests.**  `tests/wrap.rs::sigsegv_repro_79_alone` is no longer
`#[ignore]`d; `tests/wrap.rs::loft_suite` now runs
`79-null-early-exit.loft` (previously skipped via `ignored_scripts()`).
Passes debug + release, and under `target/release/loft --interpret`.

---

## 136. (historical) Wrap-suite SIGSEGV on `79-null-early-exit.loft`

**Severity:** High (release blocker — see RELEASE.md Gate Items).

**Symptom:** `cargo test --release --test wrap` (or the full suite
`./scripts/find_problems.sh`) aborts with one of:
- `free(): invalid pointer`
- `corrupted size vs. prev_size`
- `signal 11 SIGSEGV: invalid memory reference`

Always attributed to `loft_suite`, which runs every
`tests/scripts/*.loft` sequentially through `wrap::run_test`.
The wrap `loft_suite` now **skips `79-null-early-exit.loft`** via
`ignored_scripts()`, but the script is STILL covered by a
dedicated `#[ignore] sigsegv_repro_79_alone` regression test —
that test currently crashes when run (`--ignored`), locking the
reproducer for the eventual fix.

**Not** caused by this session's P54-U changes.  Still reproduces
after `git show HEAD:src/*` replaces every modified `src/` file
with its committed HEAD content.  The bug is pre-existing at
commit `d0d6932`.

**Debugger fingerprints (valgrind + crash reporter):**

```
Invalid write of size 1
   at loft::fill::op_return
   by loft::state::State::execute_argv
 Address ... is 8 bytes after a block of size 8,008 alloc'd
   by loft::state::State::new
```

In a debug build the bounds check fires earlier:

```
thread 'sigsegv_repro_79_alone' panicked at src/store.rs:902:9:
Store read out of bounds: rec=1 fld=8005 size=4 store_size=8008

=== loft crash (wrap) SIGABRT caught ===
  last op:  (opcode dispatch) (op=5)
  pc:       0
  fn:       (?) (d_nr=4294967295)
===
```

The 8008-byte block is the stack store allocated in `State::new`
(`db.database(1000)` → 1000 words × 8 bytes).  `op_return` (op=5)
writes 8 bytes past the end of that block — `stack_pos` climbs
above 8000.  Live instrumentation shows `fn_return` being called
repeatedly at `code_pos=6` (or 12 / 18), reading `u32::MAX` but
getting `6` / `12` / similar small bytecode offsets, turning the
wrap-test binary into an infinite loop that grows the stack by
12 bytes per iteration until it overflows into adjacent heap and
corrupts Rust's allocator metadata.  The `Data::drop` at end of
`run_test` then finds corrupted `Value`/`String` entries and
glibc aborts.  `call_stack` is empty by the time the loop runs
(d_nr=u32::MAX in the crash report) — execution has already
left main and is "returning past the bottom of the stack".

**Runs fine via CLI:**

```
$ target/release/loft tests/scripts/79-null-early-exit.loft
  (exits 0, clean)
$ valgrind target/release/loft tests/scripts/79-null-early-exit.loft
  (zero memory errors)
```

So the bug lives somewhere in the difference between
`cached_default()` → clone → `run_test` vs. a fresh
`parser.parse_dir` → parse user file → execute.

**Leading hypotheses (unverified):**

1. **Frame-yield residue from a default-parse side effect.**  The
   default library's parser pass registers some lazily-initialised
   state (static `NATIVE_REGISTRY`, closure maps, etc.).  If the
   cached clone differs subtly from a fresh parse — a differently-
   sized stack reserve, a const-store offset, an unset `arguments`
   register — main's `OpReturn` could read its ret/discard operands
   off the wrong bytecode position and corrupt the stack.
2. **C56 `?? return` interaction with top-level return.**  Script
   79 is the ONLY script in the suite using `?? return`.  The
   desugared form emits an inner `OpReturn` inside `safe_double`
   / `chain_test` / `void_test`.  A compile-time mismatch between
   `self.arguments` (cached at def_code entry) and the current
   stack.position at the nested `Return` could land us at wrong
   offsets on return.
3. **Stale `self.arguments` between functions.**  `self.arguments`
   is a `State` field mutated inside `def_code`.  If a previous
   def's value leaks into another def's `gen_return`, the bytecode
   for that return has the wrong `ret` operand.

**To reproduce:**

```
cargo test --release --test wrap sigsegv_repro_79_alone -- --ignored --nocapture
```

**Debug aids already in place** (no setup needed for next session):

- `src/crash_report.rs` — `install("loft")` is called from
  `src/main.rs` startup; `install("wrap")` is called from
  `tests/wrap.rs::run_test`.  The interpreter's execute loop in
  `src/state/mod.rs::execute_argv` calls `set_context(pc, op_code,
  op_name, fn_d_nr, fn_name)` at every opcode dispatch.  On
  SIGSEGV/SIGABRT/SIGBUS the handler async-signal-safely prints
  the published context to stderr, then the default handler runs
  to produce the core dump.
- `tests/wrap.rs::sigsegv_repro_79_alone` (`#[ignore]`) is the
  standalone reproducer; `tests/wrap.rs::ignored_scripts()`
  skips `79-null-early-exit.loft` from `loft_suite`.
- `ulimit -c unlimited` + `sysctl -w kernel.core_pattern=/tmp/core.%e.%p`
  — local core dumps, inspect with `gdb -c core target/release/deps/wrap-<hash>`.
- `valgrind --error-exitcode=42 --track-origins=yes --num-callers=30
  target/release/deps/wrap-<hash> sigsegv_repro_79_alone --ignored
  --nocapture` — points `op_return` at the out-of-bounds write.

**Discovered:** 2026-04-14 during P54-U phase 2 test sweep.
Reproduces at `d7ef549` (`origin/main` after PR #170 merge); was
also reproducible at the pre-merge `d0d6932` commit.
See `CHANGELOG.md` and `doc/claude/RELEASE.md` § "Crashes" for
release-block ownership.

---

## Package / Multi-file

### ~~142~~. `vector<T>` field panics when T is a struct from an imported file — FIXED

**Status:** Fixed 2026-04-17 — plain `use` now imports all `pub` definitions
via `import_all`, so `vector<T>` content types resolve correctly across files.
Related to the P173 intra-package `use`-cycle fix.  Historical detail below
kept for archaeology.

**Severity (historical):** High — used to block multi-file library layout for
any package that used `vector<StructType>` fields where the struct was defined
in a separate `.loft` file.

**Symptom:** The parser panics with:

```
assertion `left != right` failed: Unknown vector unknown(N) content type on [M]Outer.field
  left: 4294967295
 right: 4294967295
```

at `src/typedef.rs:311` during the type-fill phase (`fill_all`).

**Reproducer (minimal):**

```
# inner.loft
pub struct Inner { val: integer not null }

# outer.loft
use inner
pub struct Outer { items: vector<Inner> }
fn test_it() {
  o = Outer { items: [] };
  assert(len(o.items) == 0, "empty");
}
```

Run: `loft --lib <dir-containing-inner> outer.loft` → panic.

The identical code in a single file works without issue:

```
struct Inner { val: integer not null }
struct Outer { items: vector<Inner> }
```

**Root cause (likely):** `typedef.rs::fill_all` resolves `vector<T>` content
types during the type registration loop.  When `T` is a struct loaded via
`use` from a different file, the struct def-nr is not yet known at the point
where the vector content type is resolved — the two-pass design fills types
file-by-file, so cross-file struct references in vector generics see
`u16::MAX` (4294967295) instead of the real def-nr.

**Workaround:** Put all structs that reference each other via `vector<T>`,
`hash<T>`, `index<T>`, or `sorted<T>` in the same `.loft` file.  This is
sufficient for the Moros `moros_map` package (all types in one file).

**Discovered:** 2026-04-14 while implementing MO.1a (Moros hex scene map
data model).  The designed layout had `types.loft`, `palette.loft`, and
`spawn.loft` as separate files with `Map` referencing all of them via
`vector<T>` fields.

---

### ~~143~~. SIGSEGV returning default struct from function iterating nested vectors — FIXED

**Status:** Fixed 2026-04-15 — see "Final fix" section below.

**Severity:** High — used to crash the interpreter.

**Symptom:** `SIGSEGV caught, last op: (opcode dispatch) (op=194)` when a
function returns `Hex {}` (default-constructed struct) as a fallback after
iterating a `vector<Chunk>` where `Chunk` contains `vector<Hex>`.  The
function works correctly when called from a single-file program but
crashes when loaded via `use` from a multi-file package.

**Reproducer:**

```loft
// types.loft (imported via use)
pub struct Hex { h_material: integer not null }
pub struct Chunk { ck_cx: integer not null, ck_cy: integer not null,
                   ck_cz: integer not null, ck_hexes: vector<Hex> }

// entry.loft
use types;
pub struct Map { m_chunks: vector<Chunk> }
pub fn map_get_hex(m: Map, q: integer, r: integer, cy: integer) -> Hex {
  for gh_c in m.m_chunks {
    if gh_c.ck_cx == q / 32 && gh_c.ck_cz == r / 32 {
      return gh_c.ck_hexes[0];
    }
  }
  Hex {}   // ← SIGSEGV here
}

// test.loft
use entry;
fn test_missing() {
  m = Map { m_chunks: [] };
  h = map_get_hex(m, 5, 5, 0);   // crashes
}
```

**Workaround:** Avoid returning a default-constructed struct from functions
that iterate nested `vector<struct>`.  Use a boolean `map_has_chunk()`
guard and skip the call when the chunk is missing.

**Discovered:** 2026-04-14 while implementing MO.2 (moros_map serialization).

**Regression fixtures:** `tests/lib/p143_types.loft`,
`tests/lib/p143_entry.loft`, `tests/lib/p143_main.loft` — three IR
shapes (empty-map fallback, found-on-first-chunk, loop-fallback-after-
miss).  `tests/issues.rs::p143_default_struct_return_from_nested_vector_use`
runs the script under the interpreter and asserts `had_fatal` stays
false.  Currently `#[ignore]` until a working fix lands.

**Fix-attempt history (2026-04-15):** Commits `82a8483` + `078459f`
dropped the unconditional `0x8000` "free source" bit on
`OpCopyRecord` in `gen_set_first_ref_call_copy`
(`src/state/codegen.rs:1192-1196`) and added explicit `OpFreeRef` on
hidden ref-typed args of the call.  In release that fixed P143
(use-after-free gone, valgrind clean) but in debug the leak-check at
`src/state/debug.rs:1045` caught a per-iteration work-ref leak in
`p122_gl_collision_struct_api` — the reassignment path at
`src/state/codegen.rs:891-931` already chose `tp_val = tp_nr` when
`has_hidden_ref` is true and never freed the work-ref either.  A
follow-up that mirrored the OpFreeRef-on-hidden-ref-args loop into
the reassign path then broke `brick_buster_yield_resume` — the
explicit free of the work-ref before scope exit invalidated the
returned `Mat4`'s `m: vector<float>` field, which was deep-copied via
`OpCopyRecord` but apparently still aliased through the work-ref's
store somehow.  All three commits reverted in `ddc4a24`.

**Why the obvious fix doesn't work:** The 0x8000 path frees whatever
the callee returned, on the assumption the callee allocated a fresh
store via `__ref_1`.  That's the common case (fall-through with a
local promoted to `__ref_1` via `ref_return`).  The pathological
case is an early-return that returns a DbRef *aliasing one of the
callee's arguments* (e.g. `return gh_c.ck_hexes[0]` inside
`for gh_c in m.m_chunks` — the returned DbRef points into the
caller's `m`).  Freeing that "source" frees part of the caller's
argument.  Conversely, NOT freeing it leaks the work-ref's allocation.
Both behaviours are in the existing test suite.

**Third attempt (2026-04-15, also failed):** Tried option 3 above —
inject `OpDatabase + OpCopyRecord(returned_dbref, __ref_1, tp) +
Return(__ref_1)` at `src/parser/control.rs::parse_return` for ref/
struct-enum returns whose dep doesn't already contain `__ref_1`.
Mirror of the existing vector-return wrap at lines 2248-2266.
Two sub-issues blocked it:
  - Timing: at the time `parse_return` processes the early-return,
    the fallthrough's `Struct {}` literal (which would create the
    `__ref_1` work-ref) hasn't been parsed yet, so `__ref_1` doesn't
    exist as a variable.  Either the wrap needs to defer to a
    post-parse pass, or it needs to allocate the work-ref on demand.
  - Slot allocation: allocating `__ref_1` on demand via
    `vars.work_refs(&t, &mut self.lexer)` creates a variable but
    leaves `stack_pos = u16::MAX`.  Codegen at
    `src/state/codegen.rs:1869` does `before_stack - r` and panics
    with "attempt to subtract with overflow" because the slot
    allocator (run earlier) didn't see this var.

**Final fix (variant of option 3 above):** Instead of changing
`OpCopyRecord` to walk arguments at runtime, achieve the same effect
by *locking* the args at codegen time — `OpCopyRecord` already has a
`!locked` guard at `src/state/io.rs:1001` that skips the source-free
when the source store is locked.

`gen_set_first_ref_call_copy` in `src/state/codegen.rs` now emits, for
every Reference/Vector/Enum-struct argument of the call:

```
n_set_store_lock(arg, true)   ← lock before OpCopyRecord
... OpCopyRecord(call_result, v, tp | 0x8000)
n_set_store_lock(arg, false)  ← unlock after
```

If the callee's early-return aliased one of those args, OpCopyRecord
sees `data.store_nr` is locked → skips the free → caller's argument
stays intact.  If the callee returned a fresh allocation (its
`__ref_1` work-ref), `data.store_nr` is unlocked → free as before.
Const args are already locked from function entry; the lock op is a
no-op on them, and `n_set_store_lock(false)` on a program-lifetime
locked store (rc >= u32::MAX/2) is a no-op too — so const args don't
get their lock cleared.

Companion change: `src/scopes.rs::free_vars` now treats
`__ref_*`/`__rref_*` work-refs as freeable at function exit
regardless of their `dep` list, recovering storage that previously
leaked via `OpDatabase`'s "clear+claim into free-marked store"
path.

---

### ~~144~~. Infinite loop when `&Struct` functions call each other in cross-file packages — FIXED

Reclassified as a symptom of P145. See P145 for root cause and fix.

---

### ~~145~~. SIGSEGV / infinite loop calling user function with name colliding native stdlib — FIXED

**Root cause:** User function `to_json(m: Map) -> text` gets internal
name `n_to_json`, which collides with the native stdlib's
`n_to_json` (JsonValue serializer registered in `src/native.rs:98`).
During bytecode generation, `generate_call` looked up the function
name in `library_names` and found the native entry, emitting
`OpStaticCall` (native function dispatch) instead of `OpCall` (user
bytecode dispatch).  `OpStaticCall` reads completely different stack
arguments, causing a SIGSEGV.

The PROBLEMS.md P144 entry originally described "store mutations lost
through forwarded `&mut DbRef`" — that was a misdiagnosis of the
same underlying name collision.  The native `n_to_json` function
operates on `JsonValue`, not `Map`, so it reads garbage DbRef bytes
from the caller's stack.

**Fix:** `src/state/codegen.rs` `generate_call` now checks
`data.def(op).code != Value::Null` — if the definition has a user
body, it always uses `OpCall`, never `OpStaticCall`, regardless of
whether the name appears in `library_names`.

**Guard:** `src/state/io.rs` `format_db()` now validates store_nr and
db_tp bounds under `#[cfg(debug_assertions)]` so future store-access
bugs panic with diagnostics instead of SIGSEGVing.

**Tests:** `tests/issues.rs::p145_text_return_multivec_struct_cross_file`

**Discovered:** 2026-04-15.  Fixed: 2026-04-15.

---

### ~~152~~. Whole-replacement assignment to a struct field is silently dropped (vector field) or undetected as a write (struct field) — FIXED

**Status:** Fixed 2026-04-16.  `parse_assign_op` now rewrites `field = vec_var`
to `OpClearVector + OpAppendVector` and `field = []` to `OpClearVector` alone;
`find_written_vars` unifies `first_arg_write` so collection ops + `OpCopyRecord`
count as writes through `OpGetField` destinations.
Tests: `tests/issues.rs::p152_*` (4 regression guards, all passing, no `#[ignore]`).
Follow-up P154 closed the RHS-eval ordering edge case introduced by this
lowering.  Historical detail below kept for archaeology.

**Severity (historical):** High — silent data loss for vector/sorted/hash/index/spacial fields.

**Symptom (vector field):**

```loft
struct S { v: vector<integer> }

fn modify(s: S) {
  fresh: vector<integer> = [1, 2, 3];
  s.v = fresh;        // ← silently dropped at runtime
}

fn test() {
  s = S { v: [] };
  modify(s);
  assert(len(s.v) == 3, "got {len(s.v)}");  // FAILS: got 0
}
```

The same shape with `&S` instead of `S` is rejected at parse time as
"Parameter 's' has & but is never modified" — exposing the same root
cause from a different angle.

**Symptom (struct field):**

```loft
struct Inner { x: integer not null }
struct Outer { i: Inner }

fn modify(s: &Outer) {        // error: parameter 's' has & but is never modified
  fresh = Inner { x: 99 };
  s.i = fresh;                // works at runtime via OpCopyRecord, but ...
}                             // ... mutation detection misses OpCopyRecord
```

The struct case is a **compiler-side** false-negative on the `&` mutation
check (the runtime OpCopyRecord does the right thing).  The vector case is
**runtime data loss**.

**Matrix of variants:**

| RHS form | Result |
|---|---|
| `s.x = 99` (scalar integer) | works |
| `s.t = fresh` (text variable) | works |
| `s.i = Inner { x: 99 }` (struct inline literal) | works |
| `s.v = [1, 2, 3]` (vector inline literal) | works |
| `s.v = []` (empty vector literal) | **silent data loss** |
| `s.v = fresh` (vector variable) | **silent data loss** |
| `s.i = fresh` (struct variable) | works at runtime; `&` mutation check fails |
| `s.v += [1]` (append) | works |
| `s.v[0] = 99` (element assign) | works |

**Root cause (vector field, runtime):**

`src/parser/collections.rs::towards_set` (lines 287-308) handles
`f_type` ∈ {Vector, Sorted, Hash, Index, Spacial} by emitting a proper
`v_set` only when `to` is a plain `Value::Var`.  When `to` is a field
access (`Value::Call(OpGetField, [base, pos, info])`) the function falls
through to `return val.clone();` — returning the bare RHS with no
assignment wrapper.  parse_assign_op then writes that bare value back
into `code` and codegen runs it, evaluating the RHS but never writing
it into the field.

The same field-access shape for non-empty inline vector literals like
`[1, 2, 3]` is handled by `create_vector` upstream (it returns a
`Value::Insert` whose head builds the vector directly in the field's ref
slot), so those paths don't hit the bug.  An empty literal `[]` and a
bare `Var(vec)` both bypass `create_vector` and land in the buggy branch.

**Root cause (struct field, mutation detection):**

`src/parser/mod.rs::find_written_vars` (lines 2772-2774) marks the
first arg of `OpSet*` / `OpNewRecord` / `OpAppendCopy` calls as written.
Struct-field whole-replacement uses `OpCopyRecord` (via
`towards_set`'s reference branch on line 281 → `copy_ref`), which is not
in that list, so the `&` parameter's mutation goes undetected.

**Fix path:**

1. **Vector / sorted / hash / index / spacial field assignment**: in
   `towards_set`, replace the `return val.clone()` fallback with an
   emission of `OpSetInt(base, pos, val)` — mirroring
   `set_field_check`'s vector branch (`src/parser/mod.rs:1492-1497`).
   Detect the `OpGetField`-shaped LHS and lift its first two args.
2. **Struct field whole-replacement mutation detection**: extend
   `find_written_vars`'s `field_write` set to include `OpCopyRecord`,
   matching how `OpSet*` already mark their first-arg vars as written.

**Workarounds (until fixed):**

- For vector fields: `s.v += new_elem;` to append; `s.v[i] = x;` to
  replace an element.  To replace the whole vector, mutate elements in
  a loop, or restructure the API so the field's owner builds the new
  vector locally and returns it.
- For struct fields (compile-time only): drop the `&` and rely on
  by-reference semantics of struct args, or assign field-by-field
  (`s.i.x = 99;`) to keep mutation detection happy.

**Discovered:** 2026-04-16, while building `lib/moros_editor`'s undo
stack — `s.us_entries = rebuilt;` (vector field whole-replacement)
silently failed.

**Tests:** `tests/issues.rs::p152_*` (regression guards;
`#[ignore]`'d until fixed).

---

### ~~153~~. Vector relocation during struct-construction transfer corrupts destination storage — FIXED

**Severity:** High — silent data loss / libc `double free or corruption` abort.

**Symptom:** Building a local vector past ~186 elements and transferring it
to a struct field via `C { v_field: hexes }` corrupted the destination:

```loft
struct H { h_material: integer not null }
struct C { ck_hexes: vector<H> }

fn test() {
  hexes: vector<H> = [];
  for _ in 0..1024 { hexes += [H {}]; }   // build local vector
  c = C { ck_hexes: hexes };               // transfer to struct field
  newh = H {};
  newh.h_material = 42;
  c.ck_hexes[167] = newh;
  assert(c.ck_hexes[167].h_material == 42, "got {...}");  // ← reads null
}
```

Threshold: 187 elements (first growth past the initial-capacity block that
required `Store::resize` to relocate instead of extend in place).  Appending
after the transfer tripped `double free or corruption` in libc.

**Root cause:**

1. **`src/database/structures.rs::vector_set_size`**: after `store.resize`
   relocated the vector's block, the local `vec_rec` was not updated.  The
   final `store.set_int(vec_rec, 4, new_length)` wrote the length into the
   OLD (just-deleted) record.  The relocated record kept the freshly-allocated
   length of 0.
2. **`src/database/structures.rs::vector_add`**: `new_db` was captured from
   `vector_append`, then `vector_set_size` on the next line could relocate the
   destination.  All three byte-copy branches used `new_db.rec`, which was the
   old, freed rec.  Subsequent reads saw length 0 and returned defaults;
   later heap touches of the corrupted block aborted the process.
3. **`src/parser/objects.rs::handle_field`**: only bare-Var vector
   initializers emitted `OpAppendVector` (deep-copy); function-call initializers
   like `C { v: build() }` fell through to a plain push that left the field
   empty.

**Fix:**

1. `vector_set_size`: `vec_rec = new_vec` after the field-pointer update,
   so the length write lands in the current record.
2. `vector_add`: re-read `dest_rec` from the field slot after
   `vector_set_size`; rebuild `new_db` with the fresh rec (element offset
   is layout-stable across relocation).
3. `handle_field`: widen the deep-copy check from `matches!(value,
   Value::Var(_))` to `!matches!(value, Value::Insert(_) | Value::Null)` so
   any vector-typed expression (Var, Call, etc.) gets OpAppendVector.

**Tests:** `tests/issues.rs::p153_*` — four guards (bare-var transfer,
function-call transfer, append-after-transfer, direct-into-field control).

**Discovered:** 2026-04-16, while wiring `lib/moros_editor`'s undo stack on
top of `lib/moros_map` (whose `build_chunk` builds a 1024-element vector
before struct construction).  Fixed: 2026-04-16.

---

### ~~155~~. SIGSEGV in `OpGetVector` after push/undo/mid-assert/redo/final-read sequence — FIXED

**Status:** Fixed 2026-04-16.  The reassignment path in
`src/state/codegen.rs::generate_set` (lines 891–932) emitted `OpCopyRecord`
with the `0x8000` "free source" flag around a user-fn call, but without the
`n_set_store_lock` bracket that `gen_set_first_ref_call_copy` uses on the
first-assignment path.  When the callee returned a DbRef aliased with a
caller arg, the free-source flag freed the caller's arg store; later uses
SIGSEGV'd in `OpGetVector`.  Fix: port the lock/unlock bracket onto the
reassignment path (same pattern as P143).
Tests: `tests/issues.rs::p155_segv_undo_redo_midassert` (passing, no `#[ignore]`).
Historical detail below kept for archaeology.

**Severity (historical):** High — reliable crash.

**Symptom:** The loft interpreter aborts with SIGSEGV at `OpGetVector`
(op=194) when a function runs the shape "mutate a vector field via a
recorded snapshot, undo, read the field in an assert, redo, read the
field again".  Removing the mid-assert read makes the crash go away.

**Minimal reproducer** (22 lines, self-contained):

```loft
struct H { m: integer not null }
struct Elm { prev: H }
struct Ct { items: vector<H> }
struct Ss { undo: vector<Elm>, redo: vector<Elm> }

fn read_at(c: Ct, idx: integer) -> H { c.items[idx] }

fn test_r() {
  c = Ct { items: [H{}, H{}, H{}, H{}, H{}, H{}] };
  s = Ss { undo: [], redo: [] };
  h = read_at(c, 2);
  s.undo += [Elm { prev: h }];
  nh = H {}; nh.m = 77; c.items[2] = nh;
  e = s.undo[0];
  cur = read_at(c, 2);
  s.redo += [Elm { prev: cur }];
  c.items[2] = e.prev;
  assert(read_at(c, 2).m == 0, "reverted");   // ← removing this makes the crash go away
  re = s.redo[0];
  c.items[2] = re.prev;
  assert(read_at(c, 2).m == 77, "reapplied"); // ← SIGSEGV here
}

fn test() { test_r(); }
```

**Hypothesis:** The pattern leaves a store reference live that points
into a freed/relocated record.  The mid-assert's `read_at(c, 2)` may
produce a DbRef (returning an H from the vector) whose underlying store
gets freed before the final read, leaving the retained ref dangling.
When the final `c.items[2] = re.prev` + `read_at(c, 2)` sequence re-
enters `OpGetVector`, it dereferences a dangling DbRef.

The crash signature — op=194 (`get_vector`), path through a helper fn
that returns a struct extracted from a vector — matches the P143 family
of "DbRef into arg gets freed" issues but triggers on a different
lifecycle shape.  Likely needs a `scopes::free_vars` extension or a
`n_set_store_lock` bracket around the helper call.

**Workaround:** Inline `c.items[i]` field reads directly in the assert
instead of going through a helper that returns the struct; or drop the
mid-assert between undo and redo.

**Discovered:** 2026-04-16, while building out `lib/moros_editor`'s
redo + batch test suite.

**Tests:** `tests/issues.rs::p155_segv_undo_redo_midassert`
(regression guard; `#[ignore]`'d until fixed).

---

### ~~156~~. `vector<T>` with a struct T that shadows a stdlib constant panics the parser — FIXED

**Status:** Fixed 2026-04-16.  `parser/definitions.rs::sub_type` now checks the
element def's `DefType` before descending into the collection branch — emits
a proper diagnostic for `Constant` / `Function` / `Routine`.
`typedef.rs::fill_database` soft-continues on an unresolved vector content
type so undefined-element-type programs (`vector<Undef>`) also diagnose
cleanly instead of panicking.
Tests: `tests/issues.rs::p156_vector_element_shadows_constant` (passing, no `#[ignore]`).
Historical detail below kept for archaeology.

**Severity (historical):** Low — clean error existed for other usages of the same shadowed struct name; only the vector-element-type path was broken.

**Symptom:**

```loft
struct E { x: integer }           // E is also a loft stdlib constant
                                  // (Euler's number, default/01_code.loft:383)
struct Big { v: vector<E> }       // ← this line trips the assert
fn main() { }
```

```
thread 'main' panicked at src/typedef.rs:309:21:
assertion `left != right` failed: Unknown vector unknown(0)
content type on [544]Big.v
```

**What works** — the same `struct E` referenced any other way produces
the correct diagnostic:

- `fn main() { e = E { x: 5 }; }` → `error: struct 'E' conflicts with
  a constant of the same name`
- `struct Big { v: sorted<E[x]> }` → same clean error
- `struct Big { v: hash<E[x]> }` → same clean error

Only `vector<E>` skips the name-conflict check, registers the struct
with an unresolved type nr, and later panics in `fill_database` when
resolving the content type.

**Root cause hypothesis:** `parse_type` / the type-resolver for
`vector<T>` defers T resolution differently than the other collection
parsers.  For `sorted<T[key]>` / `hash<T[key]>` the `[key]` parse
forces a full name lookup that surfaces the conflict; for bare
`vector<T>` the lookup falls through a path that stores
`Type::Unknown(0)` and never re-checks the conflict.

**Fix path:** walk `vector<T>` through the same name-conflict detector
that fires for `struct E { … }` at the bare-use sites.  The check
should be at parse time, before `fill_database` walks the content type.

**Workaround:** rename the struct (`Elem`, `PiType`, `CharType`) — any
name that doesn't collide with a stdlib constant.  `PI` and `E` are
the currently-exposed constants (`default/01_code.loft:379`, `:383`).

**Discovered:** 2026-04-16, during moros_editor test scaffolding.

**Tests:** `tests/issues.rs::p156_vector_element_shadows_constant`
(regression guard; `#[ignore]`'d until fixed).

---

### P162 — native: `return match` with struct-enum field bindings emits `return let mut`

**Status:** fixed (2026-04-17)

**Reproducer:**

```loft
enum GShape {
  GCircle { radius: float },
  GRect { width: float, height: float }
}

fn garea(s: GShape) -> float {
  match s {
    GCircle { radius } if radius > 0.0 => PI * radius * radius,
    GCircle { radius } => 0.0,
    GRect { width, height } => width * height
  }
}
```

Native codegen produces `return let mut var__mv_radius_2: f64 = 0.0;` —
`pre_declare_branch_vars` emits variable declarations between the `return`
keyword and the if-chain.  Rust rejects `return let …` as an expression.

**Root cause:** `output_if` calls `pre_declare_branch_vars` which writes
`let mut` declarations.  When the caller already wrote `return `, the
declarations land after `return` instead of before it.  The interpreter
is unaffected; only native codegen is broken.

**Fix path:** In `output_if`, detect when inside a return context (or in
the `Value::Return` handler, hoist the if-expression into a temporary
`let` binding before `return`).  Alternatively, move `pre_declare_branch_vars`
output before the `return` keyword.

**Discovered:** 2026-04-17, pre-existing in `tests/scripts/10-match.loft`.

**Tests:** `tests/scripts/10-match.loft` native compilation (fails).

---

### P163 — `is` field capture SIGSEGV on mixed-variant loop iteration

**Status:** fixed (2026-04-17)

**Reproducer:**

```loft
enum V2 { VaText { vt: text }, VaBool { vb: boolean } }
fn test() {
  items: vector<V2> = [VaText { vt: "hi" }, VaBool { vb: true }];
  r = "";
  for it in items {
    if it is VaText { vt } { r += vt; }
    if it is VaBool { vb } { r += "{vb}"; }
  }
}
```

**Root cause:** `is`-capture bindings were placed in the condition
Insert and executed unconditionally — before the discriminant check.
When the variant didn't match, `OpGetText` on a `VaBool`'s memory
read an invalid text pointer, causing SIGSEGV.

**Fix:** Moved field-read bindings from the condition into the if-body
(via `is_capture_bindings`).  The temp subject variable (for non-Var
expressions) stays in the condition since reading the enum byte is
always safe.  Field reads now only execute when the variant matches.

**Discovered:** 2026-04-17, while converting `#fields` iteration tests.

---

### P184 — `vector<i32>` / `hash<i32>` / `sorted<i32>` ignore the `size(4)` annotation

**Severity:** Medium — silent layout mismatch between the declared narrow
element type and the actual storage.  No crash, but any binary-format
code (glTF, PNG, custom protocols) that trusts `vector<i32>` to mean
"4 bytes per element" gets the wrong file size.  Indexing also returns
values combined with adjacent elements.

**Reproducer:**

```loft
struct Box { v: vector<i32> }

fn test() {
  b = Box { v: [] };
  b.v += [1 as i32, 2 as i32, 3 as i32];
  assert(b.v[0] == 1, "v[0] = {b.v[0]}");  // FAILS: v[0] = 8589934593
  f = file("/tmp/out.bin");
  f#format = LittleEndian;
  f += b.v;
  assert(f.size == 12, "12 bytes for 3 × i32");  // FAILS: 24 bytes
}
```

The value `8589934593 = 0x200000001` is `(2 << 32) | 1` — `b.v[0]`
reads 8 bytes and gets v[0] in the low half + v[1] in the high half.

**Surfaced in:** `lib/moros_render/tests/geometry.loft::test_map_export_glb_header`
after C54 Phase 2c.  The glTF BIN chunk's triangle indices were 24
bytes per triangle (wrong) instead of the 12 bytes the header claimed.

**Root cause:**

`type i32 = integer size(4)` sets `forced_size = 4` on `i32`'s
definition.  Struct-field allocation consults it via
`src/typedef.rs::fill_database`'s `Type::Integer` arm:

```rust
let alias = data.def(d_nr).attributes[a_nr].alias_d_nr;
let s = data.forced_size(alias).unwrap_or_else(|| a_type.size(field_nullable));
```

**But the Vector arm (line 325) never consults `forced_size`** — it
uses `data.def(content_def_nr).known_type` which resolves to the
8-byte base `integer` database type for every `Type::Integer`,
regardless of the alias the user typed.  The alias info is already
collapsed by the time `Type::Vector(content, _)` is constructed in
`parse_type_full` / `sub_type`.

Same issue affects `Type::Hash`, `Type::Sorted`, `Type::Index`.

**Fix path:** full phased plan in
[`plans/02-narrow-collection-elements/README.md`](plans/02-narrow-collection-elements/README.md)
— representation choice (preferred: extend `Type::Integer` with an
`Option<NonZeroU8>` forced-size field so the alias signal flows
through `Box<Type>` naturally), then Phase 1–6 covering parser
population, resolver narrowing, read path (the hard one —
`src/parser/fields.rs::parse_vector_index`'s compile-time
`elm_size` has to look up the vector's real stride, not use a
constant), append / insert / set paths, local variables and
return types, and the hash / sorted / index extension.  Per-phase
acceptance criteria and regression-test matrix spelled out there.

**Tried and reverted** (2026-04-21): storage narrowing landed via
`Attribute.content_alias_d_nr`, but the parser's `elm_size`
computation at the indexing site stayed 8-byte — producing *worse*
behaviour than leaving the bug (4-byte storage + 8-byte reads =
garbage values).  Reverted entirely.  Detailed postmortem and the
"all or nothing" rule in the plan file above.

**Workaround:** use `vector<integer>` (8-byte elements) and add
explicit `as i32` casts at binary-write sites:

```loft
// Instead of: result: vector<i32> = []; result += [t.a]; ... f += result;
for t in tris { f += t.a as i32; f += t.b as i32; f += t.c as i32; }
```

The fix in `lib/graphics/src/glb.loft` (`glb_write_indices` helper)
uses this pattern — commits that tripped the bug replaced the
`glb_idx_buf` → `vector<integer>` helper with an inline write loop
that casts per element.

**Discovered:** 2026-04-21 while fixing `test_map_export_glb_header`
(BIN chunk double-counted bytes, tripping the header's `total_len`
assertion).

**Tests:** (see `lib/moros_render/tests/geometry.loft::test_map_export_glb_header`
for the indirect regression guard — a direct `tests/issues.rs::p184_*`
guard should land with the proper fix.)

### P185 — Slot-aliasing SIGSEGV on late local declared after inner text-accumulator loop

**Severity:** High (safety: heap corruption).  Triggers `OpFreeText`
(op 118) on a slot that still aliases a live text buffer, producing
either a raw SIGSEGV or glibc `realloc(): invalid pointer` abort on
scope teardown.  P178 / P177-class: the slot allocator placed two
locals that must not share a slot onto the same slot.

**Reproducer** (fails on clean `develop` branched from `1753615`):

```loft
fn main() {
  out = file("/tmp/out.txt");
  for f in file("tests/docs").files() {          // inline temporary iter source
    path = "{f.path}";
    if !path.ends_with(".loft") or path.ends_with("/.loft") { continue; }
    body = "";
    for i in 0..3 {
      body += "{i}";                             // grows a text buffer
    }
    key = path[path.find("/") + 1..path.len() - 5];   // declared AFTER body loop
    out += `
      {key}
    `;
    break;
  }
  println("done");
}
```

Running this prints `done` and then crashes during scope teardown
with SIGSEGV or `realloc(): invalid pointer`.  The crash is in
`free_text` (fill.rs op 118) on a slot that should already have
been handed back.

**Two independent workarounds** (either alone avoids the crash):

1. **Hoist `key` above the inner loop** — the late declaration is the
   direct trigger.
   ```loft
   key = path[path.find("/") + 1..path.len() - 5];
   body = "";
   for i in 0..3 { body += "{i}"; }
   ```
2. **Hoist `file(...)` into a named variable** — stops the allocator
   from reusing the temporary's slot.
   ```loft
   d = file("tests/docs");
   for f in d.files() { ... key = path[...]; ... }
   ```

Both nudge the slot allocator onto a non-overlapping placement.

**Root cause (hypothesis):** the scope analyser treats the inline
`file(...)` temporary as free after the iterator is materialised,
then `place_orphaned_vars` assigns `key`'s slot on top of a slot
that the text buffer (`body`) or the iterator's internal state is
still referencing via a DbRef.  When scope teardown runs `OpFreeText`
on the orphan slot, the decrement lands on a still-live store → use-
after-free.  Same family as P178 (`local_start = 0` orphan start
overlapping argument slots), which was patched with `local_start`
parameter; P185 is a different manifestation of the same "orphan
start is too aggressive" issue.

**Surfaced by:** `scripts/build-playground-examples.loft` corrupting
its own output — the script reads `tests/docs/*.loft` in an outer
`for f in file("tests/docs").files()` loop, builds `body` line-by-line
in an inner loop, then computes `key = name.to_lowercase()...` and
writes to `out` via a backtick-block append.  Result: truncated
`doc/examples.js` (2 lines of 85+) and SIGSEGV.  `make
build-playground-examples` (or any direct `loft scripts/build-
playground-examples.loft`) reproduces.

**Fix path:** the plan at `doc/claude/plans/04-slot-assignment-
redesign/` covers a broader rework of slot allocation to eliminate
the "orphan vars fall through to the wrong allocator" class of
bugs (P178, P185, and likely others).  Until that lands, the two
workarounds above are the user-facing guidance.

**Tests:** `tests/issues.rs::p185_slot_alias_on_late_local_in_nested_for`
— currently `#[ignore = "P185 — slot aliasing; see PROBLEMS.md"]`;
unignore when the fix lands.

**Discovered:** 2026-04-22, while investigating why `doc/examples.js`
drifted on the `docs-problems-sync` branch (the generator had been
quietly corrupting its own output every time it ran).

---

## See also
- [PLANNING.md](PLANNING.md) — Priority-ordered enhancement backlog
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — Language design inconsistencies and asymmetries
- [TESTING.md](TESTING.md) — Test framework, reproducing and debugging issues
- [CAVEATS.md](CAVEATS.md) — Verifiable edge cases with reproducers
- [../DEVELOPERS.md](../DEVELOPERS.md) — Debugging strategy and quality requirements
