<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 02d-iii — outer-binding rewrite design

This doc captures the pre-implementation analysis for phase
02d-iii (the major surgery of plan-22's scalar-capture boxing
arc).  Phase 02d shipped its first two foundation sub-phases on
2026-05-12; 02d-iii is structurally heavier than 02a-c combined.
Locking the surface area before any code lands is the difference
between a focused 5-commit arc and a multi-week debug loop.

## Status of plan-22 phase 02d

| Phase | What | Status |
|---|---|---|
| 02d-i | Accumulate scalar mutated-capture names onto parent function's `scalars_to_box` field | Shipped 2026-05-12 |
| 02d-ii | Synthesise canonical `__cell_<T>` structs (`{ value: T }`) for each scalar type used | Shipped 2026-05-12 |
| 02d-iii | Outer-binding rewrite — sub-steps a-e | Shipped 2026-05-12 |
| 02d-iv | Cells (b_d1/b_d2/b_d3 for float/single/character/enum + multi-scalar) + leak guard | Shipped 2026-05-12 |
| 02d-v | Boolean cells via OpEqInt LHS shape recognition in `maybe_prepend_cell_alloc` | Shipped 2026-05-12 |
| 02d-vi | Text-cell boxing (read-append-write for `log += s`; `assign_text` path needs to accept auto-deref'd LHS) | Future |

## Goal

Make this snippet print `2` on both interpreter and `--native`:

```loft
fn test() {
    n = 0;                              // boxed: allocates __cell_integer
    f = fn() { n = n + 1; };            // closure mutates n
    f();
    f();
    print("{n}\n");                     // reads via cell — expects 2
}
```

Today, with phases 02a-c + 02d-i + 02d-ii landed, the cell struct
(`__cell_integer`) and the queued name (`scalars_to_box: ["n"]`)
both exist in `Data` after parsing, but `n` is still emitted as
`OpStoreInt` / `OpVarInt` (raw stack slot).  Closure mutations
write to a private copy; outer reads see the original `0`.

## Design strategy — type-flip + universal read/write rewrite

The cleanest separation of concerns is a **two-step
transformation applied at the start of pass 2** for the parent
function:

1. **Type flip in the variables table.**  For every name in
   `Definition.scalars_to_box`, replace the variable's type
   `Integer / Text / Float / …` with `Reference(__cell_<T>_d_nr,
   vec![var_nr])` via `Variables::set_type` (no validation
   ceremony — this is an intentional incompatible flip).  After
   this flip, the variable IS a Reference local from the
   variables-table perspective; the existing
   variable-resolution and capture machinery will treat it as
   such automatically.

2. **Read-site auto-deref.**  Wherever the parser would emit
   `Var(v_nr)` for a boxed scalar (parent body OR closure body
   captured-via-closure-record), wrap the IR in
   `Call(OpGet<T>, [<dbref-yielding-IR>, Int(0)])` so the
   emitted bytecode reads the cell's `value` field instead of
   the bare DbRef.

3. **Write-site rewrite.**  When the parser would emit
   `Set(v_nr, expr)` for a boxed scalar:
   - **First assignment** (allocates the cell): emit
     `Insert([v_set(v, Null), OpDatabase(v, T_kt), OpSet<T>(Var(v),
     0, expr)])` — the same allocate-and-fill pattern
     `parse_object` already uses for struct literals
     (`src/parser/objects.rs:1306-1361`).
   - **Subsequent assignment** (writes through existing cell):
     emit `Call(OpSet<T>, [Var(v), Int(0), expr])`.
   - **Compound `n += expr`**: builds `n = n + expr` IR
     internally; the RHS reads via auto-deref (step 2), the LHS
     write goes through this rewrite (subsequent-assignment
     path).

After these three transformations the existing scope-exit free
machinery — driven by `Type::Reference(_, dep)` having a
non-empty dep — automatically frees each cell at the parent
function's scope exit, and the existing closure-capture
machinery (phase 02c) automatically wires the closure-record
attribute as auto-Reference (12-byte share-by-DbRef encoding)
because the captured type is now `Reference(__cell_<T>, [v_nr])`.

## Critical files + hook points

| What | File:line | What changes |
|---|---|---|
| **Type flip entry point** | `src/parser/definitions.rs:901` (just before `self.parse_code()`) | Add `if !self.first_pass { self.flip_scalars_to_box_types(); }` |
| **Type flip implementation** | NEW method on Parser, in `src/parser/vectors.rs` next to existing 02d helpers | Iterates `data.def(self.context).scalars_to_box`, looks up each name via `self.vars.var(name)`, flips type via `self.vars.set_type(v_nr, Reference(__cell_<T>_d_nr, vec![v_nr]))`.  Cell d_nr derived from `cell_struct_name` (already exists from 02d-ii). |
| **Read auto-deref (parent body)** | `src/parser/objects.rs:140-161` (existing local-resolution arm) | After setting `*code = Value::Var(v_nr)` and `t = self.vars.tp(v_nr).depending(v_nr)`, detect `Type::Reference(d, _)` where `data.def(d).name.starts_with("__cell_")` and re-wrap: `*code = Call(OpGet<T>, [code.clone(), Int(0)])`, `t = cell_value_type` |
| **Read auto-deref (closure body capture)** | `src/parser/objects.rs:230-235` (existing closure-record field-read arm) | Same auto-deref wrap when `t` is `Reference(__cell_*, _)` after the closure-record field read. |
| **Write rewrite (assignment)** | `src/parser/expressions.rs:716-1138` (`parse_assign_op`) | Detect LHS variable type is `Reference(__cell_*, _)` AND RHS is a scalar value (not Null, not OpCopyRecord, not Var-of-same-Reference).  Branch: first-assignment (`!is_defined`) → emit `Insert([v_set, OpDatabase, OpSet<T>(Var, 0, expr)])`; subsequent → emit `OpSet<T>(Var, 0, expr)`. |
| **Codegen first-Set arm (alternative)** | `src/state/codegen.rs:1511-1640` (`gen_set_first_at_tos`) | If the parser-level write rewrite is too invasive, add a new arm: `Type::Reference(d, _)` where `def(d).name.starts_with("__cell_")` → emit `OpDatabase(slot)` then `OpSet<T>(slot, 0, value)`.  This keeps the parser change to read auto-deref only; the assignment path stays as `Set(v, scalar_expr)` and codegen handles the cell allocation transparently. |

The two write-rewrite options trade off where the complexity
lives:

- **Parser-level rewrite (recommended)**: explicit IR shape, easy
  to reason about in dump files, no codegen surprises.  Cost:
  two parser hooks (first vs subsequent assignment).
- **Codegen-level rewrite**: one arm in `gen_set_first_at_tos`
  + one arm in the equivalent `set_var` re-assignment dispatch.
  Cost: the IR doesn't reflect the actual operation (debug-trace
  shows `Set(n, Int(0))` but bytecode does `OpDatabase +
  OpSetInt`).

Recommendation: **codegen-level rewrite for first assignment
only** (adds one arm to `gen_set_first_at_tos`),
**parser-level rewrite for subsequent assignments and the
compound `+=` case** (where `set_var` would need restructuring).
Reads always parser-level (read sites are the most numerous;
centralised auto-deref in `resolve_name` is the only sane
location).

## Hard subtleties (each one a hidden hazard)

### (1) Variable-type type check rejects scalar-on-Reference

After the type flip, the parser will see the user's source
`n = 0` with LHS type `Reference(__cell_integer, [n])` and RHS
type `Integer`.  The existing type-check machinery
(`Parser::convert` in `src/parser/operators.rs` and surrounding)
will reject the assignment as a type mismatch.

**Mitigation**: gate the type-check at the assignment site.
When the LHS is a `Reference(__cell_*, _)` AND the RHS is the
cell's value-field type, accept the assignment as the rewrite
path's input (the rewrite will desugar to a field-set, where
the type match is correct).  Hook: `parse_assign_op` before the
convert call.

### (2) `n` in capture_context vs `n` in the resolved type

At lambda-parse time, the closure captures `n`.  Today's
`capture_context` is a snapshot of `(name, type)` pairs for
every visible variable in the parent scope.  If the type flip
runs BEFORE the lambda is parsed, the snapshot sees
`Reference(__cell_integer, [n])` — correct.  If it runs AFTER,
the snapshot sees `Integer` — wrong, the closure-record
attribute would be inline 8 bytes instead of share-by-DbRef.

**Mitigation**: the type flip runs at the START of `parse_code`
for the parent function (one hook before the body walk
begins).  Lambdas inside the body are parsed AFTER the flip,
so the capture_context they snapshot already has the correct
Reference-typed `n`.

### (3) First-pass body save (phase 02a) used Integer types

Phase 02a un-gated body save in pass 1, so the IR walked by
`collect_mutated_captures` was built with Integer-typed `n`.
That's fine for the walker (which inspects op names + arg
shapes, not types).  Pass 2 doesn't replay pass-1 IR — it
re-parses from source via `lexer.body(...)`.  Net effect: the
type flip in pass 2 affects pass 2's IR generation only, which
is what we want.

**Mitigation**: nothing to do.  The split between pass-1
walking and pass-2 emit is exactly the firewall this case
needs.  Verify by reading `src/parser/definitions.rs::parse_function`
body-replay path at the landing time.

### (4) Closure body assignment `n = n + 1` LHS handling

In the closure body, `n` is captured-via-closure-record.  The
LHS of `n = n + 1` is NOT a stack-slot `Var(v_nr)` — it's
`get_field(closure_record, n_attr)` returning a DbRef.

The existing `Set(var_nr, expr)` IR shape can't represent
"write through a DbRef".  The IR shape has to be
`Call(OpSet<T>, [<dbref>, Int(0), <expr>])`.

**Mitigation**: in `parse_assign_op`, when the LHS path
resolves to a closure-captured boxed scalar, emit the
field-set call IR instead of `Set(var_nr, ...)`.  Hook the
detection BEFORE the Set IR is built — `parse_assign` already
inspects the LHS to build the IR; add a check for "captured
boxed scalar" early.

### (5) Compound assignment `n += 1` reads BEFORE writes

`n += 1` lowers to `n = n + 1` internally.  The order of
evaluation: read `n` (returns scalar via auto-deref) → add 1 →
write back (cell-set or field-set per (4)).  The auto-deref
read must produce a fresh IR snapshot for the LHS, not share
with the RHS evaluation (otherwise codegen evaluates the
cell-deref twice and gets stale-write semantics).

**Mitigation**: this is the standard compound-assign concern,
already handled by the parser elsewhere.  Verify by tracing
how `s.x += 1` for a Reference field works today (already
correct).  If today's `s.x += 1` works, then `n += 1` for boxed
`n` will work the same way (both lower to `OpSet<T>(<dbref>,
0, OpGet<T>(<dbref>, 0) + 1)` effectively).

### (6) Format-string interpolation `print("{n}\n")`

`{n}` inside a format string reads `n`'s value.  The
format-string codegen calls into the variable resolver, which
(per the read-site auto-deref hook) returns the field-read IR.
If the format-string codegen path bypasses `resolve_name` and
reads the variable's type directly, the auto-deref won't fire.

**Mitigation**: search for format-string variable-read sites
(`{` interpolation) — likely in `src/parser/expressions.rs` or
`src/state/text.rs`.  Verify they go through the standard
expression path (parse_expression / parse_primary) which calls
resolve_name.  If a direct variable-read shortcut exists,
extend it to honour the auto-deref.

### (7) `if x is Variant { x.field }` with boxed scalar

The `is` variant-check pattern doesn't apply to scalar locals
(it's enum-variant only), so no interaction.  Sanity: confirm
no edge case where a boxed scalar appears in a match arm.

### (8) Tuple destructure `(a, b) = (1, 2)` with boxed `a`

If `a` is in `scalars_to_box`, the destructure assignment hits
both the parser tuple-destructure path AND the assignment
path.  Each tuple element assignment must route through the
write rewrite.

**Mitigation**: tuple destructure already lowers to a sequence
of per-element `Set(var_i, value_i)` IR.  Each Set hits the
write-rewrite hook in `parse_assign_op`, so as long as the hook
is at the right level, this works.  Verify via T1.* test
coverage.

### (9) Scope-exit freeing — automatic via Reference dep machinery

The local's type after flip is `Reference(__cell_<T>, [v_nr])`.
The standard `get_free_vars` + scope-exit `OpFreeRef`
mechanism already handles freeing such locals at scope exit.
No new code needed.

**Mitigation**: nothing to do.  Verify via the
`p22_phase02d_…_no_leak` regression test (added as part of
02d-iv).  Any leak indicates the dep mechanism mis-fired.

### (10) `auto_build_native` artefact rebuild

After 02d-iii lands, the brick-buster + p244-style native
extension paths may need recompilation if the cell synthesis
changes the surface area (it doesn't — cells are local to the
parent function, not exported).  Sanity: `make game` after the
landing commit.

## Finding (added 2026-05-12 after 02d-iii.a) — existing void-return write-back

While shipping 02d-iii.a, attempting to wire the type flip into
`parse_function`'s pass-2 entry caused a SIGSEGV on
`p86_lambda_capture_multi_mutation` (in `tests/issues.rs`).
Diagnosis:

`src/parser/control.rs::parse_call_ref` lines 3729-3755 contains
an existing void-return-closure write-back path:

> "for void-return capturing lambdas, write updated closure record
>  fields back to the corresponding outer variables so the caller
>  observes mutations made inside the lambda body (e.g.
>  `count += x`)."

After every `add(10)` call to a void-return closure stored in a
named local, the parser emits IR that copies each closure-record
attribute back to the outer variable's slot via `v_set(outer_v,
field_val)`.  This is what makes p86's `count = 0; add = fn(x) {
count += 1; }; add(10); add(20); add(12); assert(count == 3)`
pass today.

The write-back mechanism is essentially a different boxing
strategy: closure record holds inline scalar; mutations propagate
via per-call copy-back instead of shared DbRef.  It works for
**b_d1** (closure stored in named local, void return) but NOT for
b_d2 (closure passed as fn-arg — write-back fires in the wrong
function) or b_d3 (closure stored in struct field — likewise).

**Implication for 02d-iii.a wiring**: flipping the outer scalar's
type to `Reference(__cell_<T>, [])` changes its slot shape from
8B Integer to 12B DbRef.  The write-back's `v_set(outer_v,
field_val)` then mismatches sizes → SIGSEGV.  Any pass-2 wiring
of the type flip MUST be paired with disabling/replacing the
write-back path for boxed scalars.

**Revised phasing** — 02d-iii.a ships the helper as
infrastructure WITHOUT wiring it into `parse_function`.  The
helper is invoked explicitly from tests to verify the flip
logic.  The activation moves to 02d-iii.e, AFTER 02d-iii.b-d
have wired cell-based propagation (auto-Reference closure-record
attribute + shared-DbRef reads/writes), at which point the
write-back path can be removed without regressing p86.

## Implementation phasing inside 02d-iii

The whole sub-phase is too large for one commit.  Split it
into 5 commits, each with its own regression net:

| Sub-step | What | Acceptance |
|---|---|---|
| **02d-iii.a** | Helper `flip_scalars_to_box_types` shipped as infrastructure.  NOT wired into `parse_function` yet (per the write-back finding above).  Tests invoke the helper explicitly to verify flip logic. | After explicit invocation in lib tests, the variables table shows boxed scalars with type `Reference(__cell_<T>, [])`.  Full regression net stays green (helper is dormant in production code). |
| **02d-iii.b** | Read auto-deref in `resolve_name` (both parent-body local arm + closure-body capture arm).  No write-side changes. | After parse, every `n` read in the IR is wrapped as `Call(OpGet<T>, [Var(n), Int(0)])`.  Reads work correctly post-flip; writes still broken (next sub-step). |
| **02d-iii.c** | First-assignment rewrite — adds an arm to `gen_set_first_at_tos` for `Reference(__cell_*, _)` first-Set with scalar value: `OpDatabase(slot) + OpSet<T>(slot, 0, value)`.  Plus subsequent-assignment parser-level rewrite to `Call(OpSet<T>, [Var(n), 0, expr])` in the parent body. | The canonical 02d snippet works on the interpreter: `n = 0; f = fn() { n = n + 1; }; f(); f(); print(n);` prints `2`. |
| **02d-iii.d** | Closure-body write path — `n = n + 1` inside the closure body emits `OpSet<T>(get_field(closure, n), 0, expr)` instead of `Set(var_nr, expr)`. | The canonical 02d snippet works under `--native` too (cross-mode parity). |
| **02d-iii.e** | Type-check gate in `parse_assign_op` to accept scalar RHS for boxed-scalar LHS (needed for the rewrite path's RHS to type-check).  **Activate the type flip from `parse_function` pass-2 entry.**  Remove the void-return write-back path in `parse_call_ref` (replaced by cell-based propagation from 02d-iii.b-d).  Plus regression sweep for tuple destructure / format strings / compound assign / etc. | Full regression net green: 633 issues + closure_matrix + mut_closure_matrix + leak + new b_d1/b_d2/b_d3 cells (from 02d-iv).  `p86_lambda_capture_multi_mutation` still passes via the new cell propagation (not the removed write-back). |

Each sub-commit is its own focused turn.  The whole arc is
estimated at 5 commits ≈ 5 turns based on phases 02a-c
shipping at ~1 turn each.

## Verification

After all of 02d-iii.a-e land:

```bash
# Lib tests for the new sub-phases:
cargo test --release --lib plan22

# Plan-22 cross-mode matrix (heavy):
cargo test --release --test mut_closure_matrix -- --ignored

# Full regression net:
cargo test --release --test issues
cargo test --release --test closure_matrix -- --ignored
cargo test --release --test leak

# CI gate:
cargo fmt --all -- --check
cargo clippy --release --all-targets -- -D warnings
cargo clippy --release --all-targets --no-default-features -- -D warnings
```

Acceptance for each cell of the 02d-iv matrix to be added at
the end of 02d-iii.e (per the 02d design):

- `b_d1_int_capture_local_mutates` — local integer, mutated by
  closure stored in a local — prints expected post-mutation
  value on both interp + native.
- `b_d1_text_capture_local_mutates` — text variant.
- `b_d1_multi_scalar_capture` — two different scalars boxed
  simultaneously.
- `b_d2_int_capture_arg_mutates` — closure passed as fn arg.
- `b_d3_int_capture_field_mutates` — closure stored in a struct
  field.
- `p22_phase02d_scalar_capture_no_leak` — 100-iter leak guard.

## Out of scope

- **Exotic integer widths** (u8 / i8 / u16 / i16) — phase
  02d-ii's `cell_struct_name` returns `None` for these; they
  fall through to today's stack-slot codegen.  Phase 02d-iv
  extends coverage if real-world code needs it.
- **Mutable<T> explicit boxing** (matrix row M) — separate
  phase 05.  02d-iii does NOT introduce a user-visible
  `Mutable<T>` type.
- **Case C (factory-pattern moved closures)** — phase 03,
  separate plan-22 phase.  The 02d design noted that 02d-iii's
  outer-binding rewrite is structurally heavier than Case C
  scalar boxing (which doesn't need outer-side reads to
  rewrite); the design doc proposes 02d-iii FIRST so the
  read/write rewrite infrastructure exists when 03's scalar
  variants need it.

## Risks

| Risk | Mitigation |
|---|---|
| The type flip breaks an unrelated emit site that reads `var.tp(v)` and assumes scalar.  Symptom: random codegen panic in pass 2. | 02d-iii.a (type flip alone) ships first; full regression net surfaces every site that needs adaptation BEFORE the read/write rewrites land.  Each panic gets a focused fix. |
| Closure-body write path (02d-iii.d) needs IR shape that parse_assign currently doesn't produce.  Symptom: `Set(var_nr, ...)` IR with var_nr pointing at a captured name (which has no stack slot in the closure body). | Read parser's existing capture-write path (the 02c case for Reference field-set `s.x = 7`).  If that path uses `OpSetInt(get_field(closure, s), x_pos, 7)`, the same shape works for cells with `OpSet<T>(get_field(closure, n), 0, expr)`. |
| Compound assign `n += 1` evaluates the LHS deref twice, producing inconsistent reads under non-trivial side effects (e.g. `n += f(n)` where `f` mutates).  Symptom: wrong value, hard to spot without focused tests. | Sub-phase 02d-iii.c includes a focused test `n += f(n)` with traced eval order.  If incorrect, refactor to evaluate LHS once + reuse via a temporary. |
| Format-string interpolation `print("{n}")` bypasses `resolve_name` and reads `n` directly.  Symptom: prints `0` even after closure mutations. | Step 02d-iii.b's regression sweep includes the format-string read path.  If broken, extend the format codegen to honour boxed scalars. |
| Native codegen (`--native` path) emits different code for `Reference(__cell_<T>, _)` than interp.  Symptom: cross-mode parity break. | P258 already proved native vs interp layout parity for auto-Reference.  The same `db.dbref()` field arm covers cells.  Verify via `b_d1_int_capture_local_mutates` cross-mode cell early in 02d-iii.d. |

## Cross-references

- [02d-case-b-scalar-design.md](02d-case-b-scalar-design.md) —
  pre-implementation design that scoped 02d-iii.
- [02-case-b-design.md](02-case-b-design.md) — Case B design
  that deferred 02d.
- `src/parser/vectors.rs::cell_struct_name` — phase 02d-ii.
- `src/parser/vectors.rs::accumulate_scalars_to_box` — phase
  02d-i.
- `src/parser/objects.rs::resolve_name` — capture and
  local-resolution arm.
- `src/parser/objects.rs::parse_object` — existing struct-literal
  allocate-and-fill template (lines 1306-1361).
- `src/state/codegen.rs::gen_set_first_at_tos` — first-assignment
  dispatch (lines 1511-1640).
- `src/state/codegen.rs::generate_var` — variable-read dispatch
  (lines 2401-2547).
- `src/variables/mod.rs::set_type` — type-flip API (lines
  1334-1336).
