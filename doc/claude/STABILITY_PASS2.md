<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# STABILITY_PASS2.md — relocations: every routine moves to its data structure

Pass 2 of [STABILITY_METHOD.md](STABILITY_METHOD.md), started 2026-06-11 in
the post-release low-churn window.  Scope set by the user:

- **Detect routines that operate on foreign data structures and move them to
  the module that owns the structure.**  Ownership map: `Value`/`Type`/
  `Data`/`Definition` → `src/data.rs`; `Stores` → `src/database/`; `Store` →
  `src/store.rs`; `DbRef` → `src/keys.rs`; `Function` (variable table) →
  `src/variables/`; `State` → `src/state/`.
- **Remove duplicates where already possible.**
- **Tree-walking routines are reimplemented cleanly: ONE walker on the owner
  taking a closure** (`Value::for_each_child` / `any_node`), replacing the
  hand-rolled recursive descents.  A new `Value` variant then extends ONE
  exhaustive match and every traversal inherits the edge.
- **Making the structures internally private is NOT in scope yet** — moves
  first; the privacy pass comes after.

Per-move gates: full suite + both backends; behavior-affecting unifications
(coverage-drifted duplicates) get a note on which direction widened.

## Status legend

`☐ todo` · `▶ partial` · `✅ done` · `➖ nothing foreign found`

## The walker (the keystone)

`Value::for_each_child(&mut FnMut(&Value))` — the ONE place that knows
`Value`'s tree shape (exhaustive match, no wildcard) — plus
`Value::any_node(&mut FnMut(&Value) -> bool)` (pre-order, Span-transparent)
and the shared predicates built on them (`reads_var`, `base_var`).
Hand-rolled walkers convert to closures over these; each conversion notes
whether the unified coverage WIDENED the old walker's (drifted default arms
are the #313/#330 disease — widening is deliberate and fail-safe direction
is checked per consumer).

## Work list

| File | Foreign routines / duplicates detected (survey 2026-06-11) | Action | Status |
|---|---|---|---|
| `src/scopes.rs` | ~22 hand-rolled `&Value` walkers: `value_reads_var` (pub, 3 ext. consumers), `contains_free`, `contains_alloc`(+`_unconditional`), `is_top_free`, `is_var_null_init`, `returned_var`, `expr_ends_in_return`, `escapes_value`, `guard_escapes`, `confine_reassign_safe`, `holder_retained`, `has_free_before_alloc`, `recover_backer`, `assigns_local`, `store_dead_after_block`, `binding_source`, `walk_par_safe_value`, `walk_classified`, `walk_shallow_parent_write`, `reclaim_safe`; `needs_pre_init`/`is_value_return_type` (`&Type`) | `reads_var` → `Value::reads_var`; simple boolean walkers → `any_node` closures; analysis-specific walkers stay local but derive descent from `for_each_child` (convert incrementally — each carries semantic quirks to preserve or deliberately widen) | ▶ wave 1: reads_var moved, contains_free/contains_alloc/is_top_free converted |
| `src/parser/mod.rs` | `base_var_of`, `find_capturing_fn_ref` (`&Value`); `type_carries_closure` (pub), `type_contains_tv`, `is_*_vector_element_target` ×2, `type_element_size` (`&Type`); `is_addressable` | `base_var_of` → `Value::base_var` (done); `find_capturing_fn_ref` STAYS hand-rolled — it deliberately walks only the Block/Set/Span construction shape from vectors.rs; full descent would find unrelated FnRefs and change #313 layout decisions; Type classifiers → `Type` impl later | ▶ waves 1–2 |
| `src/parser/control.rs` | `value_mentions_var` (dup of pre_eval's), `base_host_var` (dup of base_var_of), `tail_has_tuple_leaf`, `is_block_divergent`, `definitely_returns` (pub), `find_branch_terminal_var`, `tail_var`, `collect_hidden_ref_args`; `match_arm_types_unify` (`&Type`) | dups die into `Value::reads_var`/`Value::base_var`; tail/terminal walkers → `any_node`/local closures later | ▶ wave 1: both dups removed |
| `src/generation/pre_eval.rs` | `value_mentions_var` (dup), `free_op_var`, `needs_pre_eval`, `is_void_value` (pub), `create_stack_var`; `heap_shape_matches` (`&Type`) | dup dies into `Value::reads_var`; rest convert incrementally | ▶ wave 1: dup removed |
| `src/variables/validate.rs` | `slot_kind` (verbatim dup of slots_v2), `short_type` | dup dies — import `slots_v2::slot_kind` | ✅ wave 1 |
| `src/generation/coroutine.rs` | `contains_yield`, `detect_yield_from` (`&Value`); `persistent_default` (`&Type`) | `contains_yield` → `any_node` closure; rest later | ▶ wave 1: contains_yield converted |
| `src/parser/collections.rs` | `is_capturing_fnref`, `worker_returns_capturing_closure` (`&Value`); `narrow_route_for` (`&Type`) | walkers STAY — tail/return-POSITIONAL semantics, not exists-anywhere (a tail combinator is a different keystone); `narrow_route_for` → Type impl later | ▶ wave 2: audited, positional |
| `src/parser/expressions.rs` | `leaf_tuple_lhs`, `inline_ref_set_in` (`&Value`) | `inline_ref_set_in` → `any_node` closure (done); `leaf_tuple_lhs` stays — positional LHS-shape extractor | ✅ wave 2 |
| `src/parser/definitions.rs` | `type_contains_def` (`&Type`) | died into `Type::contains_def` | ✅ wave 3 |
| `src/parser/vectors.rs` | `cell_struct_name` (pub), `cell_value_type` (`&Type`) | → Type impl / data.rs (used cross-file already) | ☐ todo |
| `src/parser/objects.rs` | construction emission reaches `database.position` (fine — asks the home); `replace_record_ref` Value rewriter | rewriter needs a `map_children` (mutating walker twin) — design with the first mutating consumer | ☐ todo |
| `src/parser/operators.rs`, `fields.rs`, `builtins.rs` | call_to_set GET→SET table (stays — op-layer fact); minor Value peeks | audit in a later wave | ☐ todo |
| `src/generation/mod.rs` | `narrow_int_cast`, `default_native_value` (pub), `is_collection_field` (`&Type`); direct `data.definitions[]`/`known_type` reads | Type impls; field reads → accessors during privacy pass | ☐ todo |
| `src/generation/emit.rs` | nested local `walk` (ncc skip-free finder), `tail_is_return` (`&Value`) | `walk` → `any_node` closure (done); `tail_is_return` stays — positional | ✅ wave 2 |
| `src/generation/ops/parallel.rs` | `closure_shape`, `is_narrow_int_return`, `tuple_elem_read`, `is_by_value_scalar` (`&Type`) | → Type impls | ☐ todo |
| `src/generation/dispatch.rs` | reads `stores.types[].name` directly | accessor (privacy pass); #260's declaration move also lands here | ☐ todo |
| `src/typedef.rs` | `has_value_cycle` (Data/attrs walker); heavy direct `Definition` field mutation (it IS the type-resolution pass — co-owner by design) | `has_value_cycle` → method on `Data`; field writes stay (resolution is its job) | ☐ todo |
| `src/variables/mod.rs` | `size`/`align` (pub, `&Type`) — THE slot-size table; `work_refs*` build from Type | `size`/`align` arguably belong to Type (data.rs)… but they encode the VARIABLE-TABLE's slot model — decide with the F5 family; keep, document ownership | ☐ todo (decision pending) |
| `src/state/codegen.rs` | `emit_typed_null`, `known_type`, `add_const` (`&Type`); `ir_contains_var` (debug assert walker); Definition `.code/.variables` writes (it IS the bytecode owner of those fields) | `ir_contains_var` → `any_node` (done — old version had no Span arm, assertion strengthened); Type helpers → Type impl later | ▶ wave 2 |
| `src/ir_store.rs` / `ir_schema.rs` / `ir_read.rs` / `ir_node.rs` / `data_store.rs` | the Value/Type CODECS (serialize the whole IR) | codecs are legitimate whole-structure visitors but should derive traversal from `for_each_child` where shape-walking (vs field-encoding); F9's "derive codec from one schema" is the bigger pass-2 item here | ☐ todo (design) |
| `src/tree.rs`, `src/radix_tree.rs`, `src/vector.rs`, `src/hash.rs` | operate on `Store`/`DbRef` raw layouts — they ARE the collection algorithms over the store (co-owners by design, like fill.rs) | no move; privacy pass gives them a defined interface later | ➖ |
| `src/parallel.rs`, `src/extensions.rs`, `src/native.rs`, `src/wasm_gl.rs`, `src/data_store.rs` | 60+ direct `stores.allocations[]` touches | accessor surface on `Stores` (worker-slot, swap-back, lock APIs) — design as ONE batch with THREADING.md in hand | ☐ todo (batch design) |
| `src/state/io.rs`, `state/debug.rs`, `codegen_runtime.rs`, `repl.rs` | direct `stores.types[].parts` reads | same accessor batch | ☐ todo |
| `src/compile.rs`, `src/state/mod.rs` | minor Definition field reads | accessors during privacy pass | ☐ todo |
| `src/main.rs`, `src/lexer.rs`, `src/keys.rs`, `src/store.rs`, `src/database/*` | own their structures | — | ➖ |

## Type walker note

Wave 3 added the `Type` keystone: `Type::for_each_child` (exhaustive over
RefVar/Vector/Rewritten/Iterator/Function/Tuple children; def-nr heads are
leaves) + `Type::any_node` + `Type::contains_def`.  Remaining `&Type`
classifiers (`type_carries_closure` — prunes at the pointer marker, so it
keeps a hand-rolled match even after moving; `has_value_cycle` — a `Data`
graph walk, not a type-tree walk) move in later waves.

## Move log

### Wave 1 — 2026-06-11 — the keystone + the known duplicates

- **Added** `Value::for_each_child` (exhaustive 34-variant match, no
  wildcard), `Value::any_node` (pre-order, Span-transparent),
  `Value::reads_var`, `Value::base_var` — `src/data.rs`.
- **Died:** `scopes::value_reads_var`, `parser/control::value_mentions_var`,
  `generation/pre_eval::value_mentions_var` → `Value::reads_var`.
  *Widened:* the unified predicate also matches `TuplePut`/`FnRef` work-var/
  `FnRefDnr`/`CallRef` callee/`Iter` var/`ParFor` x_var+r_var, and descends
  `Tuple`/`Parallel`/`BreakWith`/`Iter`/`ParFor` children the mentions
  copies skipped.  Checked fail-safe per consumer: scan_set #316 (wider →
  fewer pre-frees), codegen `rhs_reads_v` (wider → stash mode), objects
  #330 hoist gate (wider → more hoisting), control guard detection (wider →
  condition correctly recognised as a guard), pre_eval
  `target_used_between` (wider → fewer return-collapses).
- **Died:** `parser/mod::base_var_of`, `parser/control::base_host_var`
  (identical bodies) → `Value::base_var`.
- **Died:** `variables/validate::{SlotKind, slot_kind}` (verbatim dup) →
  imports `slots_v2::{SlotKind, slot_kind}`.
- **Converted to `any_node` closures:** `scopes::contains_free`,
  `scopes::contains_alloc`, `coroutine::contains_yield`.  *Widened:* all
  three now descend every edge (CallRef args, If conditions, Iter, ParFor);
  for free/alloc detection wider = more reclaim exclusion (safe); for yield
  detection full coverage is the correct semantics (a missed yield
  miscompiled).  **NOT converted:** `contains_alloc_unconditional` — its
  refusal to descend If/Loop/Parallel is dominance semantics, not drift.
- Gates: full suite 2289 passed / 173 skipped (baseline-equal), clippy
  clean, fmt applied.
- Known benign: `tests/fixtures/libs/imaging/native` cdylib rebuild fails
  pre-existing (`loft-ffi-build = "0.1"` from crates-io lacks
  `generate_register_from_loft_with_bridges`; no test consumes the
  fixture — candidate for the accessor-batch wave or a fixture path-patch).

### Wave 2 — 2026-06-11 — pure exists-anywhere walkers; positional walkers audited

- **Converted to `any_node` closures:** `expressions::inline_ref_set_in`
  (drift found: `BreakWith` was a leaf — a `Set` inside its value was
  missed; wider answer = correct null-init insertion point; depth-limit
  removed, deep-nesting test now also asserts the positive case),
  `state/codegen::ir_contains_var` (debug-only first-assignment
  self-reference assert; old walker had NO `Span` arm so Span-wrapped
  self-references escaped — assertion strengthened; release suite
  unaffected, debug smoke on closure-heavy scripts clean),
  `emit::block_contains_ncc_skip_free`'s nested `walk` (now also sees a
  `__ncc_` Set inside `Loop`/call args; wider → scratch-buffer
  materialisation in more cases, the safe direction).
- **Audited, deliberately NOT converted:** `collections::is_capturing_fnref`
  + `worker_returns_capturing_closure`, `expressions::leaf_tuple_lhs`,
  `emit::tail_is_return` — all POSITIONAL (tail/last/return-position)
  semantics, not exists-anywhere; `parser/mod::find_capturing_fn_ref` —
  shape-targeted (walks only the vectors.rs construction wrappers; full
  descent would match unrelated FnRefs and change #313 layout decisions).
- Gates: full suite 2288/2289 + `moros_glb_cli_end_to_end` transient
  cdylib-build race (passes in isolation after clearing `native-auto/`;
  the known concurrent-build truncation), debug smoke on closure-heavy
  scripts, clippy clean, fmt clean.

### Wave 3 — 2026-06-11 — the Type keystone + the contains-def twins

- **Added** `Type::for_each_child` / `Type::any_node` /
  `Type::contains_def` — `src/data.rs`.
- **Died:** `definitions::type_contains_def` + `parser/mod::type_contains_tv`
  (same predicate, drifted arms, both only ever called with type-variable
  numbers) → `Type::contains_def`.  *Widened:* descends Tuple / Function /
  Iterator / RefVar / Rewritten children — `substitute_type` already
  rewrites through Tuple (plan-17), so the GET-side predicate had drifted
  behind the SET side; also matches all def-carrying heads (Routine,
  Sorted/Index/Spacial/Hash) — harmless for the tv callers (def numbers are
  unique) and honest for future ones.
- Gates: full suite 2289 passed / 173 skipped, clippy clean, fmt clean.
