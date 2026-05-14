<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 00 — Matrix freeze + cross-mode harness

**Status: shipped 2026-04-30 (harness) + filled in across phases
01-05/07 (cells).  Final matrix below reflects the closed state of
plan 14 as of 2026-05-11; 40/40 cells green on both backends.**

## Goal

Lock the element × destination matrix and ship a single test harness
that every later phase reuses.  The harness compiles a snippet under
the **interpreter** and under **`--native`**, captures both stdouts,
and asserts they are byte-identical (in addition to whatever
per-snippet assertions the snippet itself contains).

After this phase, opening a new cell test is one macro invocation;
adding a new element type or destination is one row in the matrix
table below plus one phase entry in [README.md](README.md).

No production code change in this phase — only test infrastructure
and documentation.

## The frozen matrix

Cell legend: `PASS:test_name` / `FIX:phase` / `CLOSED:reason` /
`PASS-i, FIX-n:phase` (interp passes, native fixes in named phase).

| | D1 — local var | D2 — direct stack (arg / return / inline expr / `match` subj / `if` arm) | D3 — struct field |
|---|---|---|---|
| **E1** integer/float/bool/char | PASS:`e1_d1_int_int_local`, `e1_d1_float_bool_local`, `e1_d1_char_int_local` | PASS:`e1_d2_arg_int_int`, `e1_d2_inline_get`, `e1_d2_match_subj`, `e1_d2_if_arm`, `e1_d2_return_int_int` | PASS:`e1_d3_field_int_int`, `e1_d3_field_update` |
| **E1n** `integer not null` | PASS:`e1n_d1_local` | PASS:`e1n_d2_arg` | PASS:`e1n_d3_field_intnotnull_bool` |
| **E2** text / text not null | PASS:`e2_d1_text_text_local`, `e2_d1_text_int_local` | PASS:`e2_d2_arg_text_text`, `e2_d2_inline_text`, `e2_d2_return_text_text` | PASS:`e2_d3_field_text_text` |
| **E3** nested tuple | PASS:`e3_d1_nested_local`, `e3_d1_nested_deep`, `e3_d1_text_inside`, `e3_d1_elem_elem_assign` | PASS:`e3_d2_nested_arg`, `e3_d2_nested_return` | PASS:`e3_d3_field_nested` |
| **E4** closure (`Type::Function`) | PASS:`e4_d1_closure_local`, `e4_d1_closure_call`, `e4_d1_closure_swap`, `e4_d1_capture_survives` | PASS:`e4_d2_closure_arg` | CLOSED:@P251 — native projection bug for fn-ref-tuple-in-struct-field write path; cell parks until @P251 closes |
| **E5** struct reference (`Type::Reference`) | PASS:`e5_d1_struct_ref_local`, `e5_d1_struct_ref_swap`, `e5_d1_ref_int_local`, `e5_d1_ref_text_local` | PASS:`e5_d2_struct_ref_arg`, `e5_d2_struct_ref_return` | PASS:`e5_d3_field_struct_ref` |
| **E6** "structure value" — see note below | CLOSED:folded into E5 (no inline value-struct type in loft) | CLOSED:folded into E5 | CLOSED:folded into E5 |
| **E7** vector / hash / sorted / index | CLOSED:non-goal (TUPLES.md) | CLOSED:non-goal | CLOSED:non-goal |

### Open follow-ups

Two bugs surfaced during validation are parked behind dedicated cells:

- **@P250** (loop-iteration ref-tuple aliasing) — `for { (q1, q2) =
  make_pair(pa, pb); }` reads `null` for the destructured variable
  that picked up the FIRST argument on iterations >0.  Single-call
  shapes work; the loop-iter cell waits for the dep-tracking fix.
- **@P251** (E4_d3) — native codegen rejects storing a tuple-with-fn-ref
  element into a struct field with rustc E0605 `(u32, DbRef) as i32`;
  P196's `.0` projection didn't extend to this wrapping-tuple case.

### Note on E6 — what does "structure value" mean in loft?

Today loft has no inline by-value struct type distinct from
`Type::Reference`.  A `struct Foo { ... }` declaration produces a
record laid out in a store; the "value" you pass around is a
`Reference(struct_def, dep)` — a 12-byte `DbRef`.  Tuple element E5
already covers this.  Phase 04 records this folding in TUPLES.md and
DESIGN_DECISIONS.md so future readers don't propose a separate
`Type::StructValue` to plug a hole that doesn't exist.

If a future feature introduces inline value structs (none is on the
roadmap as of 2026-05-04), a new E6 row is added and re-validated.

## Cross-mode harness

### Where it lives

`tests/common/cross_mode.rs` (new module) — exposed via the existing
`tests/common/mod.rs` re-export.  All 14-plan tests live in
`tests/tuple_matrix.rs` (new file) and consume the harness via:

```rust
mod common;
use common::cross_mode::cross_mode;
```

### Macro shape

```rust
/// Compile and run the given loft `body` (wrapped in `fn test() { … }`)
/// under the interpreter AND under `--native`.  Capture stdout from
/// both runs.  Assert:
///
///   1. interp run exits 0
///   2. native run exits 0
///   3. native_stdout == interp_stdout (byte-identical)
///
/// `body` should write its observation to stdout via `print(...)`
/// (the assert macros print on failure too — both modes produce
/// identical stdout when the assertion holds).
///
/// Reuses `tests/native.rs::run_native_job` and the existing wrap
/// harness.  Skips with `println!` (not panic) when `rustc` is
/// unavailable on the test machine — matches `tests/native.rs`
/// behaviour for CI machines without a Rust toolchain.
#[macro_export]
macro_rules! cross_mode {
    ($name:ident, $body:expr) => {
        #[test]
        fn $name() {
            $crate::common::cross_mode::run_cross_mode(stringify!($name), $body);
        }
    };
}
```

### `run_cross_mode` implementation sketch

```rust
pub fn run_cross_mode(test_name: &str, body: &str) {
    let source = format!(
        "fn test() {{ {body} }}\n\
         fn main() {{ test(); }}\n"
    );

    // 1. Interpreter run — reuse existing wrap-mode infrastructure.
    let interp_stdout = run_interp(&source);

    // 2. Native run — reuse tests/native.rs job runner.
    let Some(rlib_info) = find_loft_rlib() else {
        eprintln!("[skip] {test_name}: libloft.rlib not found");
        return;
    };
    let native_stdout = match run_native(&source, &rlib_info) {
        Ok(s) => s,
        Err(NativeUnavailable) => {
            eprintln!("[skip] {test_name}: rustc unavailable");
            return;
        }
        Err(e) => panic!("native run failed for {test_name}: {e}"),
    };

    // 3. Cross-mode equivalence — the heart of the matrix.
    if interp_stdout != native_stdout {
        let diff = pretty_diff(&interp_stdout, &native_stdout);
        panic!(
            "{test_name}: interp/native divergence\n\
             ---- interp ----\n{interp_stdout}\
             ---- native ----\n{native_stdout}\
             ---- diff ----\n{diff}"
        );
    }
}
```

### Why a new harness, not extending `wrap.rs` / `native.rs`

- `wrap.rs` runs only the interpreter and asserts against a
  per-snippet expected-stdout string.  No native run.
- `tests/native.rs` runs the native compiler on `tests/docs/*.loft`
  files but does not capture stdout for cross-comparison — it only
  checks exit status.
- The 14-plan needs both runs **on the same source** with **stdout
  comparison**.  Splicing this into either harness would force every
  caller to opt in via a parameter; a dedicated harness keeps the
  two-mode contract obvious at the call site.

The new harness re-exports the helpers from `tests/native.rs` rather
than duplicating them: `find_loft_rlib`, `compile_native_job`,
`run_native_job`.  Phase 00 lifts these to `pub(crate)` (one-line
visibility change in `tests/native.rs`) so `tests/common/cross_mode.rs`
can call them without copy-paste.

### What goes in a body

A cell test's `body` is a small loft snippet that:

1. Constructs a tuple of element type E and stores it via destination
   D.
2. Reads it back.
3. Prints the read-back value(s) in a stable, single-line format.
4. Calls `assert(...)` for the per-cell invariant.

```rust
cross_mode!(e1_d1_int_int_local, "
    t = (3, 7);
    print(\"{t.0},{t.1}\\n\");
    assert(t.0 == 3 && t.1 == 7, \"e1_d1\");
");
```

Both modes print `3,7\n` then run the assert.  Cross-mode
equivalence catches the case where the interpreter prints `3,7\n` but
native prints `0,0\n` (silent corruption — the historical pattern
behind @P200 / @P203 / @P205).

## Per-cell test inventory drafted in 00

This phase only reserves the **test names** and matrix slots — the
actual `cross_mode!` calls land in the corresponding fix-phase
commit.  The names below freeze the naming convention so phase 01–05
commits do not bikeshed.

```
e1_d1_int_int_local
e1_d1_float_bool_local
e1_d1_char_int_local
e1_d2_arg_int_int                  // call f((3, 7))
e1_d2_return_int_int               // requires-T1.8a
e1_d2_inline_get                   // (3, 7).0
e1_d2_match_subj                   // match (3, 7) { ... }
e1_d2_if_arm                       // x = if cond { (1, 2) } else { (3, 4) }
e1n_d1_local
e1n_d2_arg
e2_d1_text_text_local
e2_d1_text_int_local
e2_d2_arg_text_text
e2_d2_return_text_text             // requires-T1.8a
e2_d2_inline_text                  // ("a", "b").0
e3_d1_nested_local                 // ((1, 2), 3)
e3_d1_nested_deep                  // ((1, 2), (3, 4))
e3_d2_nested_arg
e3_d2_nested_return                // requires-T1.8a
e4_d1_closure_local                // (counter_closure, "tag")
e4_d1_closure_call                 // t.0(5) returns expected
e4_d2_closure_arg
e5_d1_struct_ref_local             // un-ignore tuple_struct_refs
e5_d1_struct_ref_swap
e5_d2_struct_ref_arg
e5_d2_struct_ref_return            // requires-T1.8a + T1.8c
e_all_d3_struct_field              // pivot test from phase 05
```

A cell with `requires-T1.8a` lands `#[ignore = "T1.8a — @PLAN06
phase 9a"]` until the @PLAN06 prerequisite ships, then the ignore is
removed in a one-line follow-up commit.

A cell with `requires-T1.8c` is the active fix subject of phase 04;
no separate ignore tag.

## Acceptance for phase 00

- New file `tests/common/cross_mode.rs` exists with `run_cross_mode`
  and the `cross_mode!` macro.
- `tests/native.rs::find_loft_rlib`, `compile_native_job`,
  `run_native_job` exposed `pub(crate)`.
- New file `tests/tuple_matrix.rs` exists with one smoke test that
  exercises the harness end-to-end:
  ```rust
  cross_mode!(harness_smoke, "print(\"42\\n\"); assert(true, \"smoke\");");
  ```
  Runs green under interp + native; asserts cross-mode equivalence on
  trivial input.
- This phase plan + [README.md](README.md) matrix match.  No FIX cell
  is filed without a phase pointer.
- `make ci` green.
- No production code changed (only test infrastructure + docs).

## Risks

| Risk | Mitigation |
|---|---|
| `find_loft_rlib` visibility change touches a load-bearing file | Smallest possible diff: `pub(crate) fn` instead of `fn`.  If reviewer pushes back, copy the helpers into `tests/common/cross_mode.rs` instead — costs ~80 lines of duplication but no API change. |
| Cross-mode comparison flakes on stdout buffering or trailing newline | Normalise both stdouts: trim trailing whitespace and normalise CRLF→LF before comparison.  Document the normalisation in `run_cross_mode` doc comment so future readers don't add stdout-format-dependent assertions. |
| Test runtime grows by ~2× per cell (interp + native compile + native run) | Native compile cache (`tests/native.rs::binary_cache_valid`) covers re-runs.  First run pays the cost once; CI parallelism absorbs the rest. |
| Harness used outside this plan and accumulates cell tests in unrelated files | The macro is generic; if it spreads, that's a feature.  No mitigation needed unless naming collisions appear (none today). |

## Out of scope

- WASM cross-mode comparison.  Browser execution is not reachable
  from the synchronous test harness; phase tests for browser-relevant
  cells will add a separate WASM smoke once `tests/wasm/` grows a
  stdout-capture path.  The matrix records WASM separately.
- Performance assertions.  Cross-mode is correctness-only.
- Comparing native against `--native-emit` source.  Out of scope for
  the plan; useful only when a divergence appears and we need to
  debug the generated `.rs`.

## Cross-references

- [README.md](README.md) — full matrix; this phase fixes its shape.
- `tests/native.rs` — donor for the native compile/run helpers.
- `tests/wrap.rs` — interpreter runner; current pattern for snippet
  tests.
- `tests/common/mod.rs` — re-export point for the new harness.
- [TESTING.md](../../../TESTING.md) — `LogConfig` + `LOFT_LOG` reference
  for harness debugging.
