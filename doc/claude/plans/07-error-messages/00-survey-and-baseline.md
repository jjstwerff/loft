<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 0 — Survey and baseline

Status: done — landed 2026-04-28

Artefacts:
- [`0a-sites.md`](0a-sites.md) — categorised inventory of 815 error
  sites across 52 source files, plus the phase-1 worklist (the
  ~22 panic-style sites in scope analysis / store / database that
  lack a `Position` today).
- [`0a-sites.txt`](0a-sites.txt) — raw `rg` output behind that.
- `tests/error_messages/cases/*.loft` — 40 short bad-program
  cases, one per failure mode in the matrix below.
- `tests/error_messages/baseline/*.expect` — captured stdout +
  stderr + exit-code goldens.  `tests/error_messages.rs` runs
  byte-for-byte; `UPDATE_GOLDEN=1` re-captures.
- [`0d-bench.txt`](0d-bench.txt) — `make bench` baseline (11
  benches × 5 columns).  Phase 1 asserts ±5 % drift.
- [`0e-rubric.md`](0e-rubric.md) — span / clarity / suggestion /
  closing-phase per case.  **13 cases silently produce wrong
  output today**; phase 4 closes 8, phase 6 closes the other 5.
  6 cases are already excellent and become anchors.

## Goal

Build the floor every later phase must improve on:

1. A complete inventory of error sites in the interpreter (parser,
   type-check, scope analysis, codegen, runtime, native-codegen
   crash).
2. A small, representative corpus of bad `.loft` programs — one per
   distinct failure mode.
3. The current rendered output for each program, captured under
   `tests/error_messages/baseline/` as goldens.

No interpreter code changes in this phase.  The goldens become the
regression reference: phases 1–6 may only change a baseline output by
making it strictly more informative (more span, clearer message,
useful suggestion) — never less.

## Steps

### 0a — Inventory error sites

One-shot ripgrep pass, results recorded in `0a-sites.md` (new, lives
in this directory):

```bash
rg -n 'diagnostic!|specific!|Level::Error|Level::Fatal|panic!|unreachable!|\.expect\(' \
   src/ \
   --type rust \
   > doc/claude/plans/07-error-messages/0a-sites.txt
```

Then group by category in `0a-sites.md`:

| Category | Files | Count | Has Position? | Notes |
|---|---|---|---|---|
| Parser — token / syntax | `src/parser/*.rs`, `src/lexer.rs` | ? | yes (`self.lexer.position`) | already span-attached |
| Parser — type check | `src/parser/expressions.rs`, `operators.rs`, `objects.rs` | ? | usually | sample message audit |
| Definitions / typedef | `src/parser/definitions.rs`, `src/typedef.rs` | ? | partial | uses `Definition.position`, not per-token |
| Scope analysis | `src/scopes.rs`, `src/variables/` | ? | partial | some `Level::Fatal` without span |
| Codegen | `src/state/codegen.rs`, `src/compile.rs` | ? | rare | most are interpreter-invariant panics |
| Runtime fault | `src/fill.rs` | ? | no | the headline gap |
| Native codegen | `src/generation/`, `src/native.rs` | ? | partial | crash_report covers SIGSEGV but not source line |

The numbers are filled in during 0a.  The `Has Position?` column is
the phase-1 worklist.

### 0b — Bad-program corpus

Create `tests/error_messages/cases/` with ~40 short `.loft` files,
one per failure mode.  Coverage matrix:

| # | File | Error category | What it triggers |
|---|---|---|---|
| 1 | `parse_unclosed_paren.loft` | parser/syntax | `(` without `)` |
| 2 | `parse_missing_semicolon.loft` | parser/syntax | `let x = 1 let y = 2` |
| 3 | `parse_unknown_keyword.loft` | parser/syntax | `funcion foo() {}` (typo) |
| 4 | `parse_unterminated_string.loft` | parser/lexer | `"abc` EOF |
| 5 | `type_assign_int_to_text.loft` | type-check | `text x = 5` |
| 6 | `type_arg_mismatch.loft` | type-check | call `foo(int)` with text |
| 7 | `type_field_unknown.loft` | type-check | `p.naem` (typo on `name`) |
| 8 | `type_method_unknown.loft` | type-check | `t.lengt()` |
| 9 | `type_unknown_struct.loft` | type-check | `Plyer { name: "x" }` |
| 10 | `type_unknown_enum_variant.loft` | type-check | `Color::Bleu` |
| 11 | `type_var_uninitialised.loft` | type-check | `int x; print(x)` |
| 12 | `type_unused_var.loft` | warning | (verify warning fires) |
| 13 | `scope_break_outside_loop.loft` | scope | bare `break` |
| 14 | `scope_continue_outside_loop.loft` | scope | bare `continue` |
| 15 | `scope_return_outside_fn.loft` | scope | top-level `return` |
| 16 | `scope_var_not_in_scope.loft` | scope | use after block exit |
| 17 | `runtime_div_by_zero_int.loft` | runtime | `1 / 0` |
| 18 | `runtime_div_by_zero_float.loft` | runtime | `1.0 / 0.0` |
| 19 | `runtime_mod_by_zero.loft` | runtime | `1 % 0` |
| 20 | `runtime_index_oob_vector.loft` | runtime | `v[100]` on len-3 |
| 21 | `runtime_index_oob_text.loft` | runtime | `t[100]` on len-3 |
| 22 | `runtime_index_negative.loft` | runtime | `v[-1]` |
| 23 | `runtime_null_deref.loft` | runtime | use null DbRef |
| 24 | `runtime_narrow_cast_overflow.loft` | runtime | `i32` ← 1e10 |
| 25 | `runtime_panic_builtin.loft` | runtime | `panic("boom")` |
| 26 | `runtime_recursion_overflow.loft` | runtime | unbounded recursion |
| 27 | `runtime_stack_overflow.loft` | runtime | deep call chain |
| 28 | `runtime_unwrap_none.loft` | runtime | option/null pattern |
| 29 | `enum_match_non_exhaustive.loft` | type-check | match missing arm |
| 30 | `match_wrong_pattern_type.loft` | type-check | match `int` with `text` arm |
| 31 | `lambda_arg_count_mismatch.loft` | type-check | wrong arity |
| 32 | `iter_wrong_collection_type.loft` | type-check | `for x in 5 { … }` |
| 33 | `struct_missing_field.loft` | type-check | `Player { name: "x" }` (missing `health`) |
| 34 | `struct_extra_field.loft` | type-check | unknown field in literal |
| 35 | `interface_method_unimpl.loft` | type-check | required method missing |
| 36 | `par_worker_writes_parent.loft` | par-safety | parent-store write inside worker |
| 37 | `par_thread_count_zero.loft` | runtime | `par(..., 0)` |
| 38 | `import_unknown_file.loft` | parser | `use missing.loft` |
| 39 | `import_circular.loft` | parser | A imports B imports A |
| 40 | `format_string_arg_mismatch.loft` | type-check | `f"{x:int}"` with text |

Each `.loft` is ≤ 8 lines and isolates one failure.  No noise.

### 0c — Capture baseline

For each case, run:

```bash
cargo run --bin loft -- tests/error_messages/cases/<name>.loft 2>&1 \
    > tests/error_messages/baseline/<name>.expect
```

The `.expect` files are committed.  A new test
`tests/error_messages.rs` iterates them and asserts byte-for-byte
match (with `UPDATE_GOLDEN=1` for intentional re-capture during phase
6).

### 0d — Bench impact baseline

`make bench` recorded into `0d-bench.txt` so phase 1 can compare
post-spans-on-IR numbers and prove no regression.  No interpretation
yet; just the raw table.

### 0e — Categorise by improvement potential

For each `.expect` file, annotate in `0e-rubric.md` (new) with:

- **Span quality**: file + line + col / line only / none / wrong file
- **Message clarity**: concrete (names types and op) / generic / cryptic
- **Suggestion potential**: yes (typo of in-scope name) / no
- **Phase that closes it**: 1 / 2 / 4 / 5 / 6

This is the worklist for the rest of the plan.  Each later phase
opens with "0e says these N cases close in this phase", and closing
them is the phase's acceptance criterion.

## Atomic landing sequence

Each row is one commit, lands independently, runs `make ci` green.

| # | Step | Test |
|---|---|---|
| 0.1 | Run the `rg` audit, write `0a-sites.md` with categorised counts | Manual review (planning artefact); commit landed |
| 0.2 | Add the 40 `.loft` cases under `tests/error_messages/cases/` | New `tests/error_messages.rs` runner asserts every case parses-or-fails-cleanly (no panics, no infinite loops); 40 cases checked in |
| 0.3 | Capture `.expect` baselines under `tests/error_messages/baseline/` | Golden test in `tests/error_messages.rs`: byte-for-byte match each `.expect`; first run via `UPDATE_GOLDEN=1` writes, subsequent runs validate; CI runs the no-update path |
| 0.4 | Record `make bench` numbers in `0d-bench.txt` | None — pure data record; phase-1's bench step references it |
| 0.5 | Annotate `0e-rubric.md` (span quality / clarity / suggestion / closing-phase per case) | None — planning artefact; reviewed in PR |

## Acceptance

- `0a-sites.md` exists with categorised counts.
- `tests/error_messages/cases/*.loft` covers the 40 cases above (or
  has a recorded reason for any drop).
- `tests/error_messages/baseline/*.expect` exists for each case and
  is committed.
- `tests/error_messages.rs` runs green (asserts every `.expect`
  matches today's output).
- `0d-bench.txt` exists.
- `0e-rubric.md` annotates every case with span / clarity /
  suggestion / phase.
- `make ci` green.
- No interpreter source changes — this phase is purely scaffolding.

## Risks

| Risk | Mitigation |
|---|---|
| The 40-case list misses a real-world failure mode | The list is a starting point; phases 4 / 6 may add cases as new `RuntimeError` kinds appear.  The acceptance bar is "covers each category with at least one case", not "exactly 40". |
| Baseline output is non-deterministic (paths, times) | `.expect` files are normalised: absolute paths replaced with `<cases>/`, any timing scrubbed, build hashes scrubbed.  Normaliser lives in `tests/error_messages.rs`. |
| `make bench` numbers drift | Phase 1 re-runs and compares to a tolerance band (±5 %), same convention as plan-06 phase 0. |

## Cross-references

- [README.md](README.md) — plan overview and phase index.
- [doc/claude/TESTING.md](../../TESTING.md) § Snapshot / golden tests
  — pattern this phase reuses for `.expect` files.
