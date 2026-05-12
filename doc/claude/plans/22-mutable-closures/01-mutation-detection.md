<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 01 — Mutated-captures detection

**Status: open**

## Goal

Implement Phase 1 of [DISCUSSION.md § Analysis sketch](DISCUSSION.md#phase-1--mutated-captures-collection):
walk each closure body's IR and identify which captured names the
body writes to.  Mark the result on the captured-name table.

**No behavioral change yet.**  The mutation flag is collected but
unused — phase 02 onwards consumes it for case classification.
This phase ships purely as foundation work that the next four
phases depend on.

## What ships

Extend `Parser.captured_names` (`src/parser/mod.rs:147`) from
`Vec<(String, Type)>` to `Vec<(String, Type, bool)>` where the
new boolean is `mutated`.  Default: `false`.

After lambda parsing synthesises the closure (right before
`synthesize_closure_record` at `src/parser/vectors.rs:432`), run
the mutation walker over the closure body's IR:

```rust
fn mark_mutated_captures(
    body: &Value,
    closure_param_var: u16,
    captured_names: &mut Vec<(String, Type, bool)>,
    data: &Data,
) {
    walk(body, |op| match op {
        Value::Call(d, args) => {
            let name = data.def(*d).name.as_str();
            // Direct field writes through the closure param.
            if matches!(name,
                "OpSetInt" | "OpSetByte" | "OpSetShortRaw" | "OpSetInt4"
              | "OpSetFloat" | "OpSetSingle" | "OpSetEnum" | "OpSetText"
              | "OpSetCharacter")
            {
                if let Some(Value::Var(v)) = args.first().map(|a| a.unspan())
                    && *v == closure_param_var
                    && let Some(Value::Int(fld)) = args.get(1).map(|a| a.unspan())
                {
                    mark_field_mutated(*fld, captured_names);
                }
            }
            // Compound desugars: OpAppendVector / OpAppendText etc.
            if matches!(name,
                "OpAppendVector" | "OpAppendText" | "OpAppendCharacter"
              | "OpClearVector" | "OpClearText"
              | "OpInsertVector" | "OpRemoveVector")
            {
                // ... same root check ...
            }
            // User fns with Impure(ParentWrite) purity, where the
            // first argument's root is a captured-binding read.
            if let Some(Purity::Impure(ParentWrite)) = data.def(*d).purity
                && let Some(captured_name) = root_captured_name(&args[0], closure_param_var, data)
            {
                mark_name_mutated(&captured_name, captured_names);
            }
            // Unknown purity — conservative-mutated.
            if data.def(*d).purity == Purity::Unknown
                && let Some(captured_name) = root_captured_name(&args[0], closure_param_var, data)
            {
                mark_name_mutated(&captured_name, captured_names);
            }
        }
        Value::Set(slot, _) => {
            // Whole-binding reassignment.
            if let Some(captured_name) = name_for_slot(*slot, captured_names) {
                mark_name_mutated(&captured_name, captured_names);
            }
        }
        _ => {}
    });
}
```

Helper functions:
- `root_captured_name(arg, closure_param_var, data)` — walks an
  argument expression to find whether its root is a closure-param
  field read.  Returns the field's name or None.
- `mark_field_mutated(fld, captured_names)` — sets the bool on
  the entry whose field index is `fld`.
- `mark_name_mutated(name, captured_names)` — sets the bool on
  the entry whose name matches.

## Test surface (phase 01)

Phase 01 ships a test-only helper that exposes the mutation flags
for inspection.  Two shapes:

**(a) Rust-level unit tests** in `src/parser/mod.rs::mutation_walker_tests`
or a new `src/parser/closure_analysis.rs::tests` mod:
- Read-only body → all captures `mutated: false`.
- `state.score = state.score + 1` body → `state` mutated.
- `count += 1` (compound) body → `count` mutated.
- `vec.push(x)` (Impure(ParentWrite) call on captured `vec`) → `vec` mutated.
- `unknown_user_fn(captured)` → `captured` conservatively mutated.

**(b) `tests/mut_closure_matrix.rs` Case A regression check** —
add `a_d1_read_only_classified_as_a` cell that:
- Runs the body cross-mode (already passing).
- Additionally calls a test-only `loft::parser::Parser::last_lambda_mutation_flags()`
  helper that returns the mutation flags for the most-recently
  parsed lambda.  Asserts all-false.

The test-only helper is gated behind `#[cfg(test)]` to keep it
out of the released binary.

## Critical files

| File | Change |
|---|---|
| `src/parser/mod.rs` | Extend `captured_names` tuple shape; add `last_lambda_mutation_flags` test helper |
| `src/parser/closure_analysis.rs` (new) | The walker + helpers |
| `src/parser/vectors.rs` | Call `mark_mutated_captures` right before `synthesize_closure_record` |
| `src/parser/objects.rs` | Update the capture-context arm to push `(name, type, false)` instead of `(name, type)` |
| `src/parser/control.rs` | Same update at `try_fn_ref_call`'s capture push |

## Verification

- All existing tests still pass (no behavioral change should be observable).
- Rust unit tests for the walker pass against synthetic IR.
- `a_d1_read_only_classified_as_a` cell green.
- `tests/closure_matrix.rs` 22 cells unchanged (regression net from plan-15).
- CI gate green.

## Risks

| Risk | Mitigation |
|---|---|
| Walker misses a write opcode (false-negative) | Phase 02's case-B lowering would silently drop mutations.  Mitigation: enumerate ALL `OpSet*`/`OpAppend*`/`OpClear*` opcodes in the walker explicitly, audit `default/01_code.loft` for the canonical list, add a Rust-level test per opcode family. |
| Walker over-flags on harmless calls (false-positive) | Phase 02 would over-eagerly lower, increasing closure-record overhead.  Mitigation: the conservative-by-default policy is intentional; phase 06 audits `default/01_code.loft` purity annotations to reduce over-flagging at the source. |
| Test-only helper leaks into release binary | `#[cfg(test)]` gate; doc_hygiene check that no public release API exposes it. |
| Tuple-shape change to `captured_names` breaks existing call sites | Mechanical update; ~10 sites grep `captured_names\.push\|captured_names\.iter` to verify all updated. |

## Cross-references

- [DISCUSSION.md § Phase 1](DISCUSSION.md#phase-1--mutated-captures-collection) — algorithm.
- `src/parser/vectors.rs:760-783` — closure synthesis (where the walker hooks in).
- `src/parser/mod.rs:147` — `captured_names` declaration.
- `src/data.rs:1327-1364, 1580` — purity machinery the walker consults.
