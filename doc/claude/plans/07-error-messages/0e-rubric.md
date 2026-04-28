<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 0e — Per-case rubric (worklist for phases 1-6)

For every `.expect` baseline captured in 0c, this table annotates:

- **Span quality** — `file+line+col` / `line+col` / `line only` / `none`.
- **Clarity** — `concrete` (names types + operation), `generic`
  (correct but vague), `cryptic` (misleading or cascade-induced).
- **Suggestion potential** — does a typo of an in-scope name exist
  that `suggest_similar` could surface?
- **Closes in phase** — which phase's acceptance criteria includes
  this case?  (1 = spans on IR; 2 = renderer; 4 = typed
  RuntimeError; 5 = suggestions; 6 = type-mismatch detail.  Phase 3
  is bytecode-pc → source line and applies cross-cutting to every
  runtime case below; not listed individually.)

Cases where the captured baseline silently produces wrong output
(no error at all) are starred ⭐ — those represent the largest
quality gap, and most close in phase 4 or 6.

| # | Case | Span | Clarity | Suggest? | Closes |
|---|---|---|---|---|---|
| 1 | parse_unclosed_paren | file+line+col ✓ | generic ("Expect token )") | n/a | 6 |
| 2 | parse_missing_semicolon | file+line+col ✓ | generic | n/a | 6 |
| 3 | parse_unknown_keyword | file+line+col ✓ | **cryptic** ("Expect token =" — fully misleading; cascade) | yes (`funcion` → `fn`) | 5 + 6 |
| 4 | parse_unterminated_string | file+line+col ✓ | concrete (good) but cascades into 2 follow-ups | n/a | 6 (cascade suppression) |
| 5 | type_assign_int_to_text | file+line+col ✓ | concrete ("Cannot write text on field S.name:integer") | n/a | already good — light phase 6 |
| 6 | type_arg_mismatch | file+line+col ✓ | concrete-ish; doesn't say *which* arg index | n/a | 6 |
| 7 | type_field_unknown | file+line+col ✓ | generic ("Unknown field") | **yes** (`naem` → `name`) | 5 |
| 8 | type_method_unknown | file+line+col ✓ | **cryptic** — 7 cascading errors from one typo | **yes** (`lengt` → `len`) | 5 + 6 |
| 9 | type_unknown_struct | file+line+col ✓ | **cryptic** ("Expect token ;" — should be "unknown type") | **yes** (`Plyer` → no Player here, but suggest catalogued types) | 5 + 6 |
| 10 ⭐ | type_unknown_enum_variant | (no error, prints `c=Color::null`) | **silent fail** | yes (`Bleu` → `Blue`) | 4 + 6 |
| 11 | type_var_uninitialised | file+line+col ✓ | **cryptic** — 5 cascading errors from one undefined var | **yes** | 5 + 6 |
| 12 | type_unused_var | file+line+col ✓ | concrete (already a Warning) | n/a | already good |
| 13 | scope_break_outside_loop | file+line+col ✓ | concrete | n/a | already good |
| 14 | scope_continue_outside_loop | file+line+col ✓ | concrete | n/a | already good |
| 15 | scope_return_outside_fn | file+line+col ✓ | generic ("Syntax error: unexpected 'return'") | n/a | 6 |
| 16 ⭐ | scope_var_not_in_scope | (no error, prints `42`) | **silent fail** — block scope leak | n/a | 1 + 6 (scope walk needs span; checker missing) |
| 17 ⭐ | runtime_div_by_zero_int | (no error, prints `result=null`) | **silent null sentinel** | n/a | 4 |
| 18 ⭐ | runtime_div_by_zero_float | (no error, prints `result=inf`) | **silent inf** | n/a | 4 |
| 19 ⭐ | runtime_mod_by_zero | (no error, prints `result=null`) | **silent null sentinel** | n/a | 4 |
| 20 ⭐ | runtime_index_oob_vector | (no error, prints `x=null`) | **silent null sentinel** | n/a | 4 |
| 21 ⭐ | runtime_index_oob_text | (no error, prints `c=`) | **silent empty** | n/a | 4 |
| 22 ⭐ | runtime_index_negative | (no error, prints `x=3`) | **silent wraparound** — most surprising | n/a | 4 |
| 23 | runtime_null_deref | file+line+col ✓ (caught at parse — `struct contains itself` rejects the test before runtime) | concrete | n/a | case rewrite needed in phase 4 — capture didn't reach runtime |
| 24 ⭐ | runtime_narrow_cast_overflow | (no error, prints raw value `small=10000000000`) | **silent — cast didn't narrow** | n/a | 4 + 6 |
| 25 | runtime_panic_builtin | source loc included in message; panic site at `src/native.rs:285` | already-decent loft loc; interpreter source still leaked | n/a | 3 (cleaner panic frame) |
| 26 | runtime_recursion_overflow | line only at `src/state/mod.rs:269` — **no loft loc** | concrete-ish ("Recursion depth limit exceeded (500)") | n/a | 3 (pc → loft source) |
| 27 | runtime_stack_overflow | (same as 26 — hits recursion depth limit before stack) | concrete | n/a | 3 |
| 28 | runtime_unwrap_none | file+line+col ✓ in panic msg; panic at `src/native.rs:261` | concrete | n/a | 3 (suppress interpreter src loc) |
| 29 | enum_match_non_exhaustive | file+line+col ✓ | concrete + names missing variant | n/a | already excellent |
| 30 ⭐ | match_wrong_pattern_type | (no error, falls through to `_` and prints `other`) | **silent — text arm in int match silently skipped** | n/a | 6 |
| 31 | lambda_arg_count_mismatch | file+line+col ✓ | concrete + names expected/actual count | n/a | already decent (unused-warn is noise) |
| 32 | iter_wrong_collection_type | file+line+col ✓ | concrete first error; 3 cascade follow-ups | n/a | 6 |
| 33 ⭐ | struct_missing_field | (no error, prints `Bob` — missing field zero-filled) | **silent zero-fill** | n/a | 6 |
| 34 | struct_extra_field | file+line+col ✓ | concrete first error; 5 cascade follow-ups | n/a | 6 (cascade suppression) |
| 35 | interface_method_unimpl | file+line+col ✓ | concrete ("'Box' does not satisfy interface 'Drawable': missing draw") | n/a | already excellent |
| 36 | par_worker_writes_parent | file+line+col ✓ | **cryptic** ("Expect token ;") — par-safety check not surfacing user-readable diagnostic | n/a | 6 (par-safety message; this is a known plan-06 path) |
| 37 | par_thread_count_zero | file+line+col ✓ | **cryptic** — 8 cascading errors from one syntax decision | n/a | 6 (cascade) |
| 38 | import_unknown_file | file+line+col ✓ | concrete ("not found — searched lib/, lib_dirs, sibling packages") | n/a | already excellent |
| 39 ⭐ | import_circular | (no error, prints `from main`) | **silent — circular import not detected** | n/a | 6 |
| 40 ⭐ | format_string_arg_mismatch | (no error, prints `hello`) | **silent — format spec `:d` ignored on text** | n/a | 6 |

## Roll-up

| Status today | Count | Notes |
|---|---:|---|
| **Already excellent** (concrete message + good span) | 6 | 12, 13, 14, 28*, 29, 35, 38 (* = panic-style, phase 3 polishes) |
| Already-decent first error, cascade noise | 6 | 4, 8, 11, 31, 32, 34 |
| Generic / cryptic but caught | 6 | 1, 2, 3, 6, 9, 15 |
| Silent fail ⭐ | 13 | 10, 16, 17, 18, 19, 20, 21, 22, 24, 30, 33, 39, 40 |
| Span attached but msg cryptic | 4 | 7, 36, 37, plus 23 (case-rewrite needed) |
| Test case needs rework | 1 | 23 (struct contains itself reroutes the test) |

## Phase-closing assignment

| Phase | Cases that close | Count |
|---|---|---:|
| 1 (spans on IR) | 16 partially | 1 |
| 4 (RuntimeError kinds) | 10, 17, 18, 19, 20, 21, 22, 24 | 8 |
| 5 (suggestions) | 3, 7, 8, 9, 11 | 5 |
| 6 (type-mismatch + cascade suppression) | 1, 2, 4, 5, 6, 8, 11, 15, 16, 30, 32, 33, 34, 36, 37, 39, 40 | 17 |
| 3 (pc → source-line; cross-cutting) | applies to 25–28 | 4 |
| Already excellent | 12, 13, 14, 29, 35, 38 | 6 |
| Case rewrite | 23 | 1 |

## Headline conclusions for plan ordering

1. **Silent runtime faults (13 cases) are the biggest user-facing
   gap.**  Phase 4 (typed `RuntimeError`) closes 8 of them in one
   pass.  The other 5 silent cases (10 / 30 / 33 / 39 / 40) are
   compile-time checks that don't fire — phase 6.

2. **Cascading errors are the second biggest gap.**  6 cases (4,
   8, 11, 31 partly, 32, 34) produce one root-cause error followed
   by 2-7 noise errors.  Phase 6 should add cascade suppression
   (skip subsequent errors at the same span ± a small window once
   a parse-recovery point is hit).

3. **Suggestions land 5 cases cheaply.**  Cases 3, 7, 8, 9, 11
   each have an obvious in-scope candidate that
   `suggest_similar` should surface.  Phase 5 is small and high-
   leverage.

4. **Phase 1 (spans on IR) has only one case it directly closes
   (16 — block scope leak).**  Its real value is enabling phases
   3 / 4 / 6 — most cases above already have a span; the IR-side
   change makes the right span reach the renderer.

5. **6 cases are already excellent today.**  These become anchors:
   any future phase that touches their .expect output must keep
   the spec-quality message.

## Hand-off

Phase 1 references rows {16} as its acceptance.
Phase 4 references rows {10 only partially, 17, 18, 19, 20, 21, 22, 24}
as its acceptance.
Phase 5 references rows {3, 7, 8, 9, 11}.
Phase 6 references rows {1, 2, 4, 5, 6, 8 cascade, 11 cascade, 15,
16 message, 30, 32 cascade, 33, 34 cascade, 36, 37 cascade, 39, 40}.
Phase 3 attaches a source line to all four runtime panics
(25, 26, 27, 28) — pc → loft span lookup.
