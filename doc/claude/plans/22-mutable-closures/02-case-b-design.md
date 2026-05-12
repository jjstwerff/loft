<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 02 — implementation design analysis (added 2026-05-12)

This doc captures the pre-implementation analysis for [phase 02
(Case B — co-scoped mutating)](02-case-b.md).  The high-level
README said "change the closure-record field type from
inline-by-value to `Reference(d, [outer_var])`" — investigation
showed that assumption was incomplete; the real implementation
requires a new storage encoding for the field.  This doc records
what was found, why the obvious approach doesn't work, and the
chosen path forward.

## What the original spec assumed

From [02-case-b.md § B/Reference](02-case-b.md#breference--user-type-captures):

> When `mutated_captures[i].type` is `Type::Reference(d, deps)`
> ..., change the closure-record field type from inline-by-value
> to `Reference(d, [outer_var])`.  Codegen for the closure body's
> field reads and writes already handles `Reference<T>` (it's the
> same pattern as a struct method's `self` parameter).

Implicit assumption: `set_field_no_check`'s Reference arm checks
the dep list and emits `OpSetRef` (4B rec pointer) when deps are
non-empty, falling back to `OpCopyRecord` (deep copy) when deps
are empty.

## What the code actually does

`src/parser/mod.rs::set_field_check`, `Type::Reference(inner_tp, _)`
arm (line 3100-3114):

```rust
Type::Reference(inner_tp, _) => {
    // The value is a 12-byte DbRef; OpSetInt would only read 4 bytes of it.
    // Copy the struct bytes into the embedded field instead.
    let type_nr = if self.first_pass { ... } else { ... };
    let field_ref = self.cl("OpGetField", &[ref_code, pos_val, type_nr.clone()]);
    self.cl("OpCopyRecord", &[val_code.clone(), field_ref, type_nr])
}
```

The dep list `_` is **ignored**.  Every `Type::Reference` field
write emits `OpCopyRecord` — deep copy of the source record's
bytes into the destination's inline storage.

Symmetrically, struct-field READS of `Type::Reference` go through
`OpGetField`, which returns a `DbRef` pointing **into the parent
record** at the field's offset (the field's bytes are inline,
not a separate allocation).

**Confirmed by probing**:
```loft
struct State { x: integer }
fn main() {
    s = State { x: 0 };
    f = fn() { s.x = 7; };  // captures s by deep-copy
    f();
    println("s.x = {s.x}");  // prints 0 — outer s unchanged
}
```

This is today's C38 behavior — value-snapshot capture.

## The three real implementation options

### Option (1): NEW storage encoding via the dep list

Extend `set_field_check`'s `Type::Reference` arm to branch on the
dep list:

- `Reference(d, [])` → today's behavior (inline storage,
  `OpCopyRecord` write, `OpGetField` read).
- `Reference(d, [v, …])` → new behavior (12-byte DbRef storage,
  `OpSetDbRef` write, `OpGetDbRef` read).

**Plus** `typedef.rs::fill_database` must size the field at 12
bytes when deps non-empty (vs the inner struct's inline size when
empty).

**Plus** every other site that switches on `Type::Reference`
field shape (codegen, native, WASM) must match.

**Plus** phase 01's mutation detection must run in **pass 1**
(or its result needs to be available in pass 1) so
`synthesize_closure_record` can pick the right shape — the
chicken-and-egg.  Requires saving body IR in pass 1 too.

**Estimated scope**: 6-10 sites updated, ~150-300 LOC, real
testing required across interp + native + WASM paths.  The
"already handles Reference<T>" assumption from the high-level
spec is FALSE for struct-field storage.

### Option (2): Auto-promotion via `Type::Vector` (single-element box)

A vector<T> field is already stored as a 4-byte u32 rec
pointer (collection header).  We could rewrite mutated Reference
captures to `Type::Vector<T>` with a single element under the
hood; the user-visible access `s.x` in the closure becomes
`s[0].x`.

Pros: reuses existing vector field infrastructure end-to-end.
No new opcodes, no codegen branches, no typedef changes.

Cons: changes the user-visible access syntax inside the closure
body.  Either the parser rewrites every `s.<field>` to
`s[0].<field>` (intrusive: every captured-Reference read in the
closure body needs rewriting), or the user writes `s[0]`
explicitly (terrible ergonomics, defeats the novice-cliff goal
plan-22 exists to solve).

**Verdict**: cute, but breaks the novice-cliff goal.  Rejected.

### Option (3): Outer-side promotion to vector + closure unchanged

Keep the closure's capture mechanism as-is (deep copy), but
auto-promote the OUTER binding `s` to a single-element vector
when phase 01 detects a mutation closure on it.  The closure's
captured copy becomes a snapshot of the vector header (4 bytes);
both inner reads/writes go through the SAME backing record.

The user writes `s.x = ...` in both outer and closure scopes;
the parser rewrites every `s.<field>` access (outer AND closure)
to go through `vec[0].<field>` semantics under the hood.

Pros: novice syntax preserved.  Existing field-write codegen
unchanged (the rewrite happens at access-resolution time).

Cons: even more intrusive than option 2 — every outer-scope
reference to `s` (not just inside the closure) needs rewriting.
Plus the existing parser machinery for `s.x` access doesn't have
a hook for "auto-redirect through vector header."

**Verdict**: highest leverage on novice ergonomics but requires
the most parser surgery.  Best long-term answer; out-of-scope
for a single phase 02 commit.

### Comparison

| Approach | Scope | User syntax | Pass-ordering | Recommended? |
|---|---|---|---|---|
| Option 1: dep-driven storage | Medium-Large (typedef.rs + codegen branches) | Unchanged | Needs pass-1 detection | YES (chosen) |
| Option 2: vector wrapper | Small (parser only) | Closure body becomes `s[0].x` (BREAKING) | Pass-2 OK | NO |
| Option 3: outer-side promotion | Largest (every `s.<field>` site) | Unchanged | Pass-2 OK | Long-term, but too big for one phase |

## Chosen approach: Option 1 with sub-phasing

Phase 02 splits into three sub-commits to keep each landable
independently:

### 02a — pass-1 mutation detection (foundation) — SHIPPED 2026-05-12

Move phase 01's mutation walker to run in pass 1 too.  Requires:

- Save lambda body to `data.def(d_nr).code` in pass 1 (currently
  pass-2-only at `src/parser/expressions.rs:397`).  Verify
  nothing else depends on the pass-2-only gate.
- Run `collect_mutated_captures` in pass 1 right BEFORE
  `synthesize_closure_record`, so the closure record's attribute
  types can be picked based on the mutation flags.
- The walker's pass-1 IR shape uses direct local-Var refs (not
  closure-param threaded); update the walker's pattern matching
  if needed.
- No behavior change yet — mutation flags are stored but not
  consumed.

Phase 02a is the "save body in pass 1 + run walker in pass 1"
foundation.  Ships when the existing regression net stays green
AND the 5 walker tests in
`plan22_phase01_mutation_detection_tests` still pass.

### 02b — new storage encoding for mutated Reference captures — SHIPPED 2026-05-12

Implement the auto-Reference storage:

- New helper on `Definition`: `attribute_is_auto_reference(f_nr)`
  returns true when the attribute's type is `Reference(d, deps)`
  with non-empty `deps`.  (The dep list IS the marker.)
- `set_field_check`'s `Type::Reference` arm branches:
  - empty deps → today's `OpCopyRecord` (deep copy).
  - non-empty deps → `OpSetDbRef` (12-byte DbRef store).
- `get_field` (and any other Reference-field-read sites) branch
  similarly: empty deps → `OpGetField` (inline pointer);
  non-empty deps → `OpGetDbRef` (12-byte DbRef load).
- `typedef.rs::fill_database` sizes auto-Reference fields at
  12 bytes (DbRef size) instead of inline-struct-size.

Phase 02b ships when the existing regression net stays green AND
a hand-crafted manual test (where the closure record's attribute
type is set to `Reference(d, [v])` by direct mutation in a
Rust-level test) shows the field reads/writes use the new
opcodes.

### 02c — wire 02a's mutation flags through to 02b's encoding

`synthesize_closure_record` consults `data.def(lambda_d_nr).mutated_captures`
(from 02a) and, for each mutated Reference capture, adds the
attribute with `Type::Reference(d, [outer_var_nr])` instead of
`Type::Reference(d, [])`.

Phase 02c ships when:
- The probe snippet (captured struct with `s.x = 7`) prints
  `s.x = 7` after the closure runs.
- Three `b_d1`/`b_d2`/`b_d3` Reference-capture cells in
  `tests/mut_closure_matrix.rs` go from "today's silent failure"
  to green.
- All 22 closure_matrix + 6 mut_closure_matrix Case A cells stay
  green (regression net).
- A leak guard in `tests/leak.rs` confirms the auto-Reference
  storage doesn't leak the captured struct.

### B/Scalar (deferred to a separate sub-phase 02d)

Primitive captures (Integer/Text/Float/etc.) need a hidden cell
allocated separately and routed through.  This is a DIFFERENT
mechanism from Option 1 — primitives don't have a DbRef shape to
share.  Requires:

- Allocate a `__cell_<i>` record (1 attribute of the primitive
  type) co-located with the closure record.
- Outer binding's value flows INTO the cell at closure
  construction time (the cell becomes the canonical storage).
- All outer-scope reads/writes of the variable get rewritten to
  route through the cell.
- Closure body's reads/writes of the capture get rewritten
  similarly.

This is closer to option 3 above (outer-side promotion).  Defer
to 02d after 02a-c land and we have practical experience with
the encoding.

The EventLoop / TTT v6 server driver mostly uses struct captures
(world: World, etc.) — 02a-c is enough for that use case.
Primitive-capture mutability is a separate ergonomic win.

## Open questions / risks

| Open question | Mitigation |
|---|---|
| Saving body in pass 1 may break other passes that assume body=Null in pass 1 | Phase 02a runs the full test suite (633 issues + 47 wrap + 22 closure_matrix + 6 mut_closure_matrix + leak + native) to surface side-effects.  If anything breaks, narrow to only-save-lambda-body, not every fn body. |
| Native + WASM codegen paths for new auto-Reference opcodes may not exist | Phase 02b lands on interp first; native + WASM follow in 02b sub-sub-commits if codegen surface needs extension.  Cells gate per backend. |
| dep-list semantics is overloaded — already used for lifetime tracking | The dep-list-as-storage-marker semantics is added; lifetime tracking is preserved (deps still mean "this value depends on these vars for liveness").  Storage choice is a new READ of the same data, not a write.  Symmetric: any non-empty deps signal shared storage. |
| Outer-side mutation tracking — when closure mutates `s.x`, outer reads of `s.x` after must see 7 | With auto-Reference, the closure's `s` and the outer's `s` point to the SAME store record.  Outer reads via OpGetField(s, x_off) see the live value.  No outer rewrite needed for this case.  (Contrast with B/Scalar which DOES need outer rewrite.) |
| Drop ordering — closure-record drop must NOT free the shared struct record | The dep `[outer_var]` on the closure record's attribute tells `get_free_vars` to suppress the free.  Same mechanism as P227's text-buffer dep keeping closure work-var alive.  Confirmed by plan-15 phase 03/04 leak guards. |

## Verification gate for each sub-phase

- **02a**: 633 issues + 47 wrap + 22 closure_matrix + 6 mut_closure_matrix all green; 5 walker tests still pass; the `mutated_captures` field is populated in pass 1 (verified by a new Rust-level test).
- **02b**: same regression net green; a manual Rust-level test that constructs a closure-record attribute with `Reference(d, [v])` and verifies the emitted opcodes are `OpSetDbRef`/`OpGetDbRef`.
- **02c**: same regression net green; the probe snippet prints `s.x = 7`; 3 new `b_d1`/`b_d2`/`b_d3` cells green; leak guard clean.

## What this design intentionally defers

- Multi-position case-D diagnostics (phase 04 territory).
- Liveness analysis for case C (phase 03 territory).
- `Mutable<T>` stdlib helper (phase 05).
- B/Scalar (primitive captures via hidden cell) — sub-phase 02d, after 02a-c proves the pattern.
- Native + WASM auto-Reference codegen (sub-commits of 02b if needed).
- TTT v6 server retrofit (phase 06).

## Cross-references

- [02-case-b.md](02-case-b.md) — original phase 02 design (high-level).
- `src/parser/mod.rs::set_field_check` — the existing storage decision (line 3100-3114).
- `default/01_code.loft::OpSetRef` / `OpSetDbRef` / `OpGetDbRef` — the opcodes phase 02b uses.
- [LIFETIME.md § Function](../../LIFETIME.md) — closure-record dep semantics.
- [plan-15 phase 03/04 leak guards](../finished/15-closure-validation/00-matrix.md) — pattern for verifying no leak under shared storage.
