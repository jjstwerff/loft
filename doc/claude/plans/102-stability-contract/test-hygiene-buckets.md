<!-- Copyright (c) 2026 Jurjen Stellingwerff -->
<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->

# @PLN102 arc-E test-hygiene — the tolerated-warning buckets (measured 2026-07-19)

> The step-0 MEASUREMENT for [test-hygiene-warnings.md](test-hygiene-warnings.md): per
> `is_runtime_warning` clause (`tests/testing.rs`), the exact `code!` fixtures that would
> newly fail if that clause were removed. Method: bypass the filter
> (`is_runtime_warning && env(MEASURE_WARNINGS).is_err()`), run the 8 `code!` suites, bucket
> each failure by the warning in its `Found '…'` panel. **114 fixtures** carry a filtered
> warning, across only **2 live families** — the other 7 clauses are dead (0 fixtures).

## Summary — 7 of 9 clauses are removable with ZERO test delta

| Clause (`testing.rs`) | Family | Fixtures | Action |
|---|---|---|---|
| `:541`–`:544` | may-produce-null (÷, %, `v[i]`, `s[i]`) | **0** (emitter DN1-gated off) | delete clause, zero-delta |
| `:545` | `not null` field-read hint (`Warning: field `) | **0** (emitter DN1-gated off) | delete clause, zero-delta |
| `:563`/`:564` | N-Store nudge (`… is stored into` / `` `null` is stored ``) | **0** (live emitter, no fixture trips it) | delete clause, zero-delta |
| `:551` | redundant-`&` (`` `&` on parameter ``) | **12** | drop the `&` per fixture, then delete clause |
| `:556` | `not null` is deprecated | **110** | delete `not null` from the `.loft` per fixture, then delete clause |

Ladder (from the design): step 1 = delete the 7 zero-delta clauses (one commit); step 2 =
redundant-`&`; step 3 = `not null`; step 4 = delete the mechanism; step 5 = meta-fixture lock.

## redundant-`&` — `:551` — 12 fixtures

Fix = **drop the redundant `&`** on the parameter (field only read/mutated, never reassigned);
keep the `&` + `.warning(...)`-assert only where it deliberately exercises the RefVar path.

### `tests/issues.rs` (11)

- `p160_nested_field_vec_element_as_ref_param`
- `p160_vec_element_as_ref_param`
- `p170_placeholder_conditional_then_reassign`
- `p170_struct_placeholder_then_vec_elem_reassign`
- `p176_recursive_self_call_terminates`
- `p176_ref_param_method_style_mutation`
- `p176_transitive_forwarding_three_levels`
- `p178_is_capture_slot_alias`
- `p179_ref_field_arg_corrupts_sibling`
- `pln40_const_reassign_via_ref_rejected`
- `pln87_link_l6_struct_param_field_writes_back`

### `tests/slot_v2_baseline.rs` (1)

- `p178_is_capture_body`

## `not null` is deprecated — `:556` — 110 fixtures

Fix = **delete `not null`** from the `code!` source (a deprecated no-op; the field is
non-null by default). EXCEPTION — a fixture that exercises not-null-*dependent* behaviour
keeps `not null` and `.warning("… `not null` is deprecated …")`-asserts it; the known one is
`p285_genuine_redundant_check_still_warns` (asserts the redundant-check-on-not-null warning).

### `tests/issues.rs` (95)

- `enhancement_ref_vector_loop_mutation_detected`
- `enhancement_ref_vector_readonly_loop_still_flags`
- `gl_combined_game_loop_stress`
- `issue_313_closure_field_invoked_cross_fn_invoker_first`
- `issue_318_closure_into_argument_struct_rejected`
- `issue_318_local_closure_struct_passed_down_still_works`
- `issue_318_returning_closure_carrying_struct_rejected`
- `issue_318_vector_of_closure_carrying_struct_rejected`
- `issue_323_factory_closure_reference_capture_survives_reuse`
- `issue_323_in_frame_reference_capture_still_works`
- `issue_330_degenerate_self_assignment`
- `issue_330_self_reading_literal_reassignment`
- `issue_332_nullable_narrow_field_null_roundtrip`
- `issue_334_not_null_byte_keeps_full_range`
- `issue_334_nullable_byte_field_null_roundtrip`
- `issue_83_hash_value_field_renamed_works`
- `mid_body_nested_call_return_value`
- `nullable_receiver_field_null_check_no_warning`
- `p117_text_param_struct_return_loop_no_leak`
- `p120_field_overwrite_once`
- `p120_field_overwrite_short_loop`
- `p120_field_overwrite_twice`
- `p120_local_overwrite_in_loop`
- `p120_multi_node_transform_update`
- `p120_struct_return_in_conditional_in_loop`
- `p120_vector_field_in_returned_struct_round_trip`
- `p122_gl_collision_struct_api`
- `p122_long_running_struct_loop`
- `p122_struct_literal_in_loop`
- `p122_struct_nested_loop`
- `p122_struct_return_conditional_loop`
- `p122_struct_return_in_loop`
- `p152_struct_field_ref_param_mutation_undetected`
- `p153_vec_field_append_after_transfer`
- `p153_vec_field_direct_into_field_still_works`
- `p153_vec_field_transfer_relocation_from_call`
- `p153_vec_field_transfer_relocation_from_var`
- `p155_segv_undo_redo_midassert`
- `p158_trailing_comma_enum_variant`
- `p160_nested_field_vec_element_as_ref_param`
- `p160_vec_element_as_ref_param`
- `p161_for_over_ref_vector`
- `p164_trailing_comma_enum_variant_list`
- `p165_enum_annotation_with_variant_rhs`
- `p170_placeholder_conditional_then_reassign`
- `p176_recursive_self_call_terminates`
- `p176_transitive_forwarding_three_levels`
- `p178_is_capture_slot_alias`
- `p179_ref_field_arg_corrupts_sibling`
- `p180_int_widens_to_long_field`
- `p180_single_literal_into_float_field`
- `p181_inline_field_access_format_string`
- `p184_hash_sorted_narrow_key_field`
- `p186_struct_typed_block_expressions`
- `p187_struct_scalar_field_corrupted_after_sibling_vector_alloc`
- `p188_local_var_hash_pluseq_struct_literal`
- `p188_local_var_index_pluseq_struct_literal`
- `p188_local_var_index_scale_50_elements_unrolled`
- `p188_sorted_local_via_plus_equals`
- `p188_struct_field_hash_pluseq_struct_literal`
- `p188_struct_field_index_pluseq_struct_literal`
- `p190_local_var_sorted_iteration`
- `p191_struct_field_index_iteration_after_layout_fix`
- `p192_len_hash_struct_field`
- `p192_len_index_struct_field`
- `p193_local_var_hash_init_then_loop_add`
- `p193_local_var_index_init_then_loop_add`
- `p193_local_var_index_read_before_write`
- `p277_local_hash_pluseq_multi_literal`
- `p277_local_index_pluseq_multi_literal`
- `p277_local_sorted_mixed_scalar_and_literal`
- `p277_local_sorted_pluseq_empty_literal`
- `p277_local_sorted_pluseq_multi_literal`
- `p277_local_sorted_pluseq_single_literal`
- `p279_forward_fn_via_intermediate_local`
- `p279_forward_text_fn_into_struct_field`
- `p285_genuine_redundant_check_still_warns`
- `p285_hash_lookup_null_no_spurious_warning`
- `p293_narrow_key_hash_lookup`
- `p295_hash_index_reassign`
- `p295_sorted_reassign_from_loop_local`
- `p300_hash_return_assign_typed`
- `p300_hash_return_assign_untyped`
- `p300_index_return_assign`
- `p300_sorted_return_assign`
- `p300_var_rhs_alias_first`
- `p326_iterator_of_struct_for_loop`
- `p326_iterator_of_struct_manual_next`
- `p4d_b_par_over_sorted_via_materialise`
- `p54_b3_float_not_null_direct_return`
- `p54_b3_float_via_intermediate`
- `pass2_arity_growth_forward_caller`
- `pass2_arity_growth_forward_chain`
- `pass2_arity_growth_mutual_recursion`
- `pass2_arity_growth_self_recursive`

### `tests/parse_errors.rs` (5)

- `all_paths_return_not_null`
- `direct_return_not_null`
- `gh253_bang_on_not_null_warns`
- `implicit_return_not_null`
- `missing_return_not_null`

### `tests/slot_v2_baseline.rs` (5)

- `match_with_arm_bindings`
- `p122r_par_loop_with_inner_for`
- `p178_is_capture_body`
- `parent_refs_plus_child_loop_index`
- `struct_block_return_non_text`

### `tests/threading_chars.rs` (4)

- `par_hash_input_t4`
- `par_index_input_t4`
- `par_sorted_input_t4`
- `par_struct_to_keyed_collection_t4`

### `tests/expressions.rs` (1)

- `not_null_element_assignment`

