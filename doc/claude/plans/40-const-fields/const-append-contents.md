<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Enhancement — `const` allows `+=` append as contents mutation

**Status: DONE (2026-07-16), on `tuxedo-work`.**  Implemented as designed: `op`
threaded into `validate_write`; a compound op (`+=`) on a const collection/text
field is allowed (in-place append), while `=` (any type) and a compound op on a
scalar stay rejected.  Verified against the boundary matrix below on both backends
(append produces correct value + length; scalar `+=` and all reassigns rejected);
guards in `tests/scripts/40-const-fields.loft` + `pln40_const_scalar_compound_rejected`.
Landed together with a false-positive fix (below).  Skipped the env-gate/stdlib-A/B
ceremony — the change can only turn a compile ERROR into a success and no passing
program had a const `+=`, so it cannot regress anything.

**Also fixed here — a shipped-const false positive:** `validate_write` used the
`parent_tp` that the RHS parse overwrites, so `arr[i] = … + e.const_field` (reading a
const field on the RHS of an element-store) was mis-flagged as reassigning that
field.  Fixed to use the LHS target's parent (`lhs_parent_tp`, saved before the RHS
parse).  This is what had forced `GEdge`/`SubPath` to be reverted in the uptake.

## The gap

A `const` collection field already allows an **element** write but rejects an
**append**, even though both mutate the field's existing container in place:

```loft
struct Mesh { const verts: vector<Vertex> }
t.verts[0] = v;        // ALLOWED (element write — contents mutation)
t.verts += [v];        // REJECTED — "cannot reassign const field 'verts'"
t.verts = [v];         // REJECTED (correct — this rebinds the field)
```

That is inconsistent: `[i]=` and `+=` are both contents mutation (they change the
vector's elements/length without pointing the field at a *different* vector), yet
one is allowed and the other rejected.  The rejection is an accident of lowering,
not a deliberate semantic choice (see Root cause).

**Why it matters:** `+=`-grown accumulators are the dominant library builder shape
— `Mesh.verts`, `Scene.meshes`, `Args.options`, `Renderer.vaos`, `CellSnap.*`,
`EdgeCosts.*`.  The const uptake had to leave every one of them non-const.  Allowing
append roughly doubles how much const a library can carry — directly serving the
owner policy (see the `libs-maximize-const-fields` memory).

## Root cause

`validate_write` (`src/parser/expressions.rs:3449`) rejects a const field for **any**
write whose target resolves to the field's byte offset.  It receives only
`(to, parent_tp)` — **not the assignment op**, and it does not consult the field's
type.  So it cannot tell:
- `t.v = […]` (`op == "="`) — a genuine rebind → REJECT, from
- `t.v += […]` (`op == "+="` on a collection) — an in-place append → should ALLOW.

Element writes (`t.v[0] = x`) slip through only because their write target resolves
to the *element*, not the field's offset — so they never match the const check.
`+=` on a field resolves to the field itself, so it is caught.

## The invariant

> `const` rejects rebinding a field to a *different whole value*; it allows mutating
> the field's *existing* container in place.  For a collection/text field: element
> writes AND `+=` append are contents mutation (allowed); `=` reassignment is a
> rebind (rejected).  For a scalar field: `=` and `+=` both change the whole value
> (both rejected — a scalar has no "contents").

## The fix (localized to one chokepoint)

Thread the assignment `op` into `validate_write` (the caller `parse_assign_op` at
`expressions.rs:1402` already has it).  In the const arm, reject iff:
- `op == "="` (rebind, any field type), OR
- `op` is compound (`+=`, …) AND the field type is **scalar** (a value change).

Allow a compound op when the field type is a **collection** (`Vector`/`Sorted`/
`Hash`/`Index`/`Radix`) or **text** — that is an in-place append.  The field's type
is already in hand at the check (`attributes()[f_nr].typedef`).

**Blast radius is small** (much smaller than literal-expected-type): the only code
affected is const-field write validation.  A field must be `const` to be touched at
all, and only a const *collection/text* field's `+=`/append changes behaviour — no
existing program can regress (nothing was const before this arc; append on a
non-const field is unaffected).

## Safe small steps

| # | Step | Verify | Why safe |
|---|---|---|---|
| 0 | **Boundary matrix (extend step-4's).**  One `/tmp` probe per (field type × op): `const` scalar/text/vector/hash field under `=`, `+=`, and `[i]=`, plus construct + read.  Hand-compute each expected cell.  Prove the harness can fail (a non-const control that runs). | Records the CURRENT verdicts (append rejected) and the TARGET verdicts (append allowed on collection/text, still rejected on scalar). | No code change |
| 1 | **Thread `op` into `validate_write`** (signature change only; behaviour identical — still reject all writes). | Builds; suite green; step-0 matrix unchanged. | Pure plumbing, no logic change |
| 2 | **Relax the const arm**, gated (`LOFT_CONST_APPEND`, default OFF): allow compound op on collection/text fields; keep rejecting `=` (any) and compound on scalars. | Gate ON: the step-0 matrix flips to TARGET (append allowed on collection/text, scalar `+=` still rejected, `=` still rejected) on **both backends**. Gate OFF: unchanged. | Gated → zero default change |
| 3 | **Re-run the const uptake's `+=` cases.**  On a lib, mark a `+=`-grown field `const` (e.g. `Mesh.verts`); with the gate ON it must now compile + pass; with the gate OFF it must still error. | hex_world / graphics `+=` fields become const-able under the gate, tests green both backends. | The consumer proof |
| 4 | **Full suite + stdlib, gate ON vs OFF.**  Confirm no non-const program shifts (append on non-const is untouched) and no const program regresses. | Gate-ON suite == gate-OFF suite, both backends. | Isolates any surprise |
| 5 | **Flip default-on**, delete the gate; graduate the matrix to `tests/scripts/40-const-fields.loft`; update LOFT.md / loft-write skill / the plan matrix (drop the "`+=` rejected" note added 2026-07-16). | `make ci` green both backends; docs match. | Only after 0–4 prove it clean |

## Edge cases to pin in step 0

- **Text append** `const s: text; t.s += "x"` — text is scalar-shaped but append is
  in-place contents growth.  Decide: allow (treat like a collection append — the
  consistent choice) vs reject.  Recommendation: **allow** (same "contents, not
  rebind" logic).
- **Keyed insert** `const h: hash<…>; t.h += [entry]` — a hash/sorted/index insert.
  Same bucket as vector append → allow.
- **Element write must stay allowed** (`t.v[0] = x`) — it already bypasses the check;
  confirm the op-threading doesn't accidentally route it through the reject arm.
- **`-=` / other compound ops** — only `+=` (append) is contents on collections; any
  compound that would *shrink or rebind* (if such exists) stays rejected.  Enumerate
  loft's actual compound-assign ops in step 0.

## Interactions

- **Widens [const-suggest-lint.md](const-suggest-lint.md)'s candidate rule.**  Once
  append is allowed, a `+=`-grown field is no longer disqualified — the lint should
  suggest const on accumulators too.  Land this BEFORE flipping that lint default-on.
- **Independent of [literal-expected-type.md](literal-expected-type.md)** — different
  chokepoint, can ship in either order.

## See also

- `src/parser/expressions.rs:3449` (`validate_write`) — the const chokepoint to extend
- `src/parser/expressions.rs:1097` (`parse_assign_op`) — where `op` is known and passed
- [README.md § step 4 boundary matrix](README.md) — extend it with the `+=` row
- the `libs-maximize-const-fields` owner-policy memory — the motivation
