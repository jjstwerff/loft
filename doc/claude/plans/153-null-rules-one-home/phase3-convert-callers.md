# Phase 3 — the 61 `convert` callers, classified for the `convert_store` / `convert` split

Read from the call shape at `convert_callers.txt` (2026-09-05, tree 5bcd5678).  STORE = a
value meets a SLOT it will live in (the `⇐` the (N-Store) rule is about); TEST = a coercion
to `Boolean` or to a key/index type for a lookup; DISCHARGE = the `??` machinery's own
conversions; INTERNAL = `convert` recursing on itself; READ = must be read before deciding.

| class | count | sites |
|---|---|---|
| STORE — assignment | 7 | expressions.rs 2973 (null into field), 3301, 3444, 3532, 3650, 4874 (`parse_assign_op_inner`), 5750 (tuple-place RHS) |
| STORE — field / default | 5 | expressions.rs 1753 (`field_store_mismatch`), definitions.rs 4708, 4725 (field default), 5002 (`default_value_fn`), objects.rs 4888 (`handle_field`, struct literal) |
| STORE — parameter / argument | 4 | definitions.rs 2358 (parameter default), control.rs 14855 (fn-ref call args), operators.rs 1738 (`parse_part` args), mod.rs 11838 (`process_call_args`) |
| STORE — return / tail | 4 | control.rs 1918 (`block_result`), 13571 (`parse_return`), operators.rs 2332 (`build_null_coalesce_return`'s `ret_val`), mod.rs 8487 (generic default null into `tp`) |
| STORE — collection element | 2 | vectors.rs 4283, 4317 (`parse_item`; one direction each — READ which) |
| TEST — to Boolean | 12 | collections.rs 3477, 6176; control.rs 8620, 15192; mod.rs 4388 (`convert_condition`), 6203; operators.rs 685 (`null_test`), 1232, 1247, 2313, 2358; vectors.rs 344, 357, 3112 |
| TEST — key / index coercion | 11 | fields.rs 968, 1653, 2051, 2097, 2178, 2196, 2308, 2521, 2561, 2636, 2650 |
| TEST — cast / parse | 3 | operators.rs 3425 (`handle_operator`, the `as` cast), objects.rs 2366, 2393 (parse fns' text operand) |
| DISCHARGE — `??` default | 3 | operators.rs 2707, 2767, 2874 (`build_null_coalesce_default_inner`: `d ⇐ τ` — (N-Coal)'s own `⇐`, where a `τ?` default makes the RESULT `τ?`, measured in phase 1) |
| INTERNAL | 8 | mod.rs 4448, 4451, 4469, 4502, 4510, 4540, 4668 (`convert` on itself) + control.rs 1588 (`un_ref` — READ: a deref delivering into a slot?) |

Counts: 22 STORE (+2 parse_item to confirm) · 26 TEST · 3 DISCHARGE · 8 INTERNAL · 2 READ.

**What the split does to each class.**  STORE callers move to `convert_store(code, is,
should, what)` — the refusing face; `what` is the string the eleven `n_store_violation` asks
spell today, so those asks are deleted and their message lives once.  TEST callers keep
`convert` — a `τ?` tested against null or coerced to a key is not a store, and refusing there
would break the null-CHECK idiom (`convert`'s own comment).  DISCHARGE: the `??` default's
`⇐` is a store of `d` into the result slot — under (N-Coal) a `τ?` default is admitted and
makes the result `τ?`; so it is `convert` (admit), and the refusal surfaces where the RESULT
is stored (the local: measured).  INTERNAL: `convert` recursing keeps its own face; the
store face wraps it once at the top.

**Silent-omission check (step 2).**  After the split, a site that reaches a slot through
`convert` instead of `convert_store` is a HOLE — but a `grep` for `self.convert(` in a
store-shaped caller is one query, and the classification above is the review.  The old
name is not retired (26 legitimate callers), so the loud-omission cure is partial: the
guard is the table, kept in the plan, plus the Stage A matrix.  Weaker than a compile
error; stronger than twelve hand-spelled asks.
