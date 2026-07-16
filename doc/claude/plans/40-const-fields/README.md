<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN40 — `const` struct fields (write-once-at-construction)

Extend the existing `const` keyword from locals + parameters to
**struct fields**, giving loft a "frozen after construction" field
modifier.  Closes the locals-vs-fields asymmetry surfaced in
[INCONSISTENCIES.md § 33](../../INCONSISTENCIES.md#33-const-applies-to-locals-and-parameters-but-not-fields).

## Status

**Not started — design ready, landing site already exists.**  Single feature,
no cross-arc dependencies.  Effort: M (parser + one type-check hook + an IR-store
round-trip bit; no new opcodes, no runtime change, no store-schema change).

The parser **already recognises `const` in struct-field position and rejects it**
with a pointer to this plan (`@P386` guard, `src/parser/definitions.rs:2467`).  So
step 1 of the build is not "teach the parser a new keyword" — it is "replace that
rejection with a flag set".  That makes the whole feature an additive, opt-in
change: no existing program contains a `const` field today (they can't — it errors),
so enforcement can only ever fire on newly-written code.

## Goal

```loft
struct Token {
  const id:        integer,   // set once at construction, non-null by default
  const issued_at: integer,
  payload:         text       // mutable, as today
}

t = Token { id: 42, issued_at: 1_000_000, payload: "" };
t.payload = "hello";          // OK
t.id = 99;                    // ERROR: cannot reassign const field 'id'
```

The constraint is **purely static** — the runtime layout is unchanged, the field
read/write ops (`OpGetInt`, `OpSetInt`, …) are unchanged.  Enforcement is the
rejection of a field-write whose target attribute is `const`, at the same
parse-time hook that already rejects writes to key fields.

> **Syntax note (2026-07 refresh).**  `not null` is now **deprecated and a
> no-op** — a field type is non-null by default; write `T?` for a nullable field
> (`src/parser/definitions.rs:11`).  The examples below use the current syntax.
> The original plan (2026-05-11) was written before that change and used
> `const id: integer not null` throughout; that spelling still parses (the
> `not null` is a warned no-op) but is no longer the way to write it.

## The one design decision: a dedicated `const_field` flag

`Attribute` (`src/data.rs:2384`) already has two boolean flags that look reusable
but are **both taken**, and reusing either would break unrelated code:

| Flag | Current meaning | Why it can't carry const-field |
|---|---|---|
| `mutable` (default `true`) | `false` marks a **key field** (`set_mutable`, `typedef.rs:931`) *and*, on operator/method attributes, a bytecode-constant param that codegen **skips pushing** (`data.rs:4026` hazard note; `codegen.rs:2763`/`3020`; `create.rs:208`). | Setting `mutable=false` on a data field would (a) reuse the "key field, create a record instead" message and (b) risk the construction/codegen path skipping the field's value. |
| `constant` (default `false`) | `true` marks a **`virtual`/`computed` field** — "only return the default" (`definitions.rs:2880`). | Already means computed-no-storage; a `const` field has real storage written once. |

So add a new, orthogonal flag: **`Attribute.const_field: bool`** (default `false`).
It records exactly one fact — "this field is write-once" — read at exactly one
kind of site (the field-write guard).

## Where it plugs into the existing code

Four short touch-points, each already has a sibling doing almost the same thing:

1. **Representation** — `Attribute` in `src/data.rs:2384`; default set in
   `add_attribute` (`data.rs:3627`, where `mutable:true, constant:false` are set).
2. **Parse (struct)** — `parse_struct` field loop, `src/parser/definitions.rs:2461`.
   The `const` keyword is consumed today at line 2467.  Set the flag after
   `parse_field` returns, mirroring how the enum loop applies `#lexeme`
   (`definitions.rs:322`).
3. **Enforce** — `validate_write` (`src/parser/expressions.rs:3449`), called from
   the field-assignment path (`expressions.rs:1402`, guarded `var_nr == u16::MAX`).
   It already maps a write's byte-offset back to the field
   (`f.position == pos`) and rejects `!mutable` (key) fields.  A const-field check
   is one more `else if` there — it has the attribute identity the raw op-emit
   path (`call_to_set_op`, `operators.rs:397`) lacks (that path sees only
   `host_ref` + byte offset).
4. **Construction is automatically exempt** — a struct literal lowers via
   `Value::Insert` / `set_field_no_check` (`src/parser/objects.rs:2458`, `2831`),
   a **different** code path that never calls `validate_write`.  "Write-once-at-
   construction" falls out for free: the guard lives on the reassignment path only.
   (Step 4 still proves this with the matrix — it is a claim to verify, not assume.)

The precedent to copy is the existing const-**parameter** enforcement: the flag
`Variables::is_const_param` (`variables/mod.rs:1537`), the shared message
`"Cannot modify {} '{}'; remove 'const' or use a local copy"`, and — most
relevant — the constructor-into-const guard that branches on
`matches!(code, Value::Insert(_))` (`expressions.rs:2422`).  Const fields need the
field-keyed analogue of exactly that logic.

## Implementation — safe small steps

Each step compiles and keeps the full suite green on its own, so each is a
separate commit.  Steps 1–2 are inert (a flag nothing reads yet); step 3 only
*relaxes* an existing error; step 4 adds enforcement that can only fire on the new
syntax.  The risk is back-loaded and opt-in.

| # | Step | Where | Verify | Why safe / reversible |
|---|---|---|---|---|
| 0 | **Baseline the boundary.**  Write the positive + negative `.loft` probes (see Test strategy) and run them on **both backends**.  Today every `const`-field program fails at the declaration (`@P386`).  Record that — it proves the harness can fail. | `tests/scripts/40-const-fields.loft` (staging in `probes/` first) | All probes error at parse today | No code change |
| 1 | **Add the flag.**  `Attribute.const_field: bool`; default `false` in `add_attribute`; add to the `#[allow(clippy::struct_excessive_bools)]` note. | `src/data.rs:2384`, `:3627` | `make ci` green | A new `false` bool nothing reads is inert |
| 2 | **Round-trip the flag.**  Add a new `ATTR_CONST_FIELD` slot after the existing `ATTR_*` constants; write it in `write_attribute` and read it back in `materialize_attributes`; extend the round-trip assert; add it to the JSON attribute view. | slot in `src/data_store.rs:236`; write `src/ir_store.rs:238` + read `src/ir_store.rs:260` (`materialize_attributes`) + assert `:1535`; JSON view `src/ir_schema.rs:872` | `make ci` green; a save→reload of a struct keeps `const_field=false` | Round-trip of an always-`false` value is identity; required so a **cached library struct** keeps its const-ness (the 2026-05 plan predates this round-trip — do not skip it) |
| 3 | **Accept + store the keyword.**  At `definitions.rs:2467` replace the `@P386` rejection with `let is_const = self.lexer.has_keyword("const");`, then after `parse_field` (`:2490`) set `attributes[idx].const_field = is_const`.  Reject `const virtual(...)` here (virtual already implies no-write).  Apply the same to enum-variant fields (`parse_enum_values` loop, `definitions.rs:286`). | `src/parser/definitions.rs` | `const x: integer` now parses and runs; the flag is set (unit assert on the `Attribute`); `const virtual(...)` errors | Only behaviour change is `const` no longer erroring — the intended direction.  Nothing reads the flag yet, so no write is rejected → zero false positives |
| 4 | **Enforce reassignment.**  Extend `validate_write` (`expressions.rs:3449`) with an `else if …const_field` arm emitting `"cannot reassign const field '{f}' of struct '{T}' — const fields are write-once-at-construction"`.  Use the **boundary matrix** (below) to find every write route that must be covered; add the field-keyed guard on any route `validate_write` doesn't reach (e.g. collection-field reassignment via the `towards_set` collection branch, `collections.rs:794`). | `src/parser/expressions.rs`, possibly `src/parser/collections.rs` | Full matrix: every negative cell errors, every positive cell (construction + read) still works, **both backends** | Fires only when `const_field==true`, which only step-3 code can set → no existing program affected.  This is the load-bearing step; gate it on the written matrix, not the suite alone |
| 5 | **Construction completeness.**  Confirm a non-null const field with no default is already required at construction (the existing non-null construction check covers it) and that a `const x: integer?` with no default is legal (stays null).  Add a message only if a gap shows. | `src/parser/objects.rs` (`object_init`) | Positive/negative construction probes pass | Reuses existing non-null enforcement; likely a test-only step |
| 6 | **Document.**  `const` in the field-modifier list. | `doc/claude/LOFT.md § Field modifiers`, `.claude/skills/loft-write/SKILL.md` | `gendoc`; doc drift check | Docs only |
| 7 | **Dogfood.**  Pick a current real consumer with the **rebuild-via-construction** shape — a struct whose instances are replaced wholesale (`grid[idx] = Cell{…}`) rather than mutated in place — and mark those write-once fields `const`.  Construction keeps working; an accidental in-place field write is now caught.  (Candidates drift; check `loft install` / `lib/*` for a grid/cell world at build time — do not hard-code a path.) | a `lib/*` consumer chosen at build time | Consumer runs unchanged; a deliberate in-place write errors | Real consumer that proves the rule earns its keep; revert if it fights the code |
| 8 | **Close the gap.**  Flip INCONSISTENCIES § 33 to resolved; graduate the probes to `tests/issues.rs::p386_const_field_*` (the id already exists as the parser guard) + `tests/scripts/40-const-fields.loft`. | `doc/claude/INCONSISTENCIES.md`, `tests/` | `make ci` | Closes the loop; regression guard lands with the feature |

**Total effort:** M — one focused session.  No new opcodes, no runtime code, no
store-schema change beyond the one round-trip bit in step 2.

## Enforcement surface — the boundary matrix for step 4

The check must fire on **reassignment** and stay silent on **construction and
read**, for every field type and every write route.  Build this in throwaway
`/tmp` probes on `--interpret` first, hand-computing each expected cell, then
verify on `--native`:

| Field type | Construct (must PASS) | Read (must PASS) | Reassign field (must ERROR) | Mutate contents (out of scope — must PASS) |
|---|---|---|---|---|
| `const x: integer` | `T{ x: 1 }` | `t.x` | `t.x = 2` | — |
| `const s: text` | `T{ s: "a" }` | `t.s` | `t.s = "b"` | — |
| `const r: OtherStruct` | `T{ r: O{…} }` | `t.r.f` | `t.r = O{…}` | `t.r.f = 5` (f not const) |
| `const v: vector<integer>` | `T{ v: [1] }` | `t.v[0]` | `t.v = [2]` | `t.v[0] = 9` (Rust `let v` rule) |
| `const x: integer?` (nullable) | `T{}` (stays null) | `t.x` | `t.x = 1` | — |

Write routes that must all hit the ERROR cells (the plan's "fires regardless of
access route"): direct (`t.x = …`), nested (`box.t.x = …`), through a `&`
parameter (`fn f(t: &T){ t.x = … }`), and a closure capture (`|| t.x = …`).  Each
lowers the LHS through the same field-assignment path, so `validate_write` should
see them all — **confirm, don't assume**.  A no-error cell is vacuous unless a
sibling cell proves the probe can error.

## What `const` does NOT cover

- **Contents through a const collection/struct field.**  `const v: vector<integer>`
  freezes the *binding* (`t.v = […]` rejected) but not the elements
  (`t.v[0] = 5` allowed) — same as Rust's `let v = vec![…]; v[0] = 5;`.  Do **not**
  put the check in `set_field_check` (`mod.rs:5227`): that chokepoint is shared by
  construction, element deep-copies, and ~20 compiler-synthesised loop/vector
  writes, so `emit_check` is not a clean "user re-assignment" signal.
- **Mutation through `Reference<T>`** — `Reference<T>` bypasses normal field-write
  checks (it points into the heap).  Const stays a static, zero-cost check; it does
  not extend to reference writes.  Document as an explicit limitation.
- **Enum-variant match bindings** (`if shape is Circle { radius }`) are already
  read-only locals — const-by-construction, no annotation needed.

## Open questions (not blocking)

- **Should a const field auto-imply non-null?**  Moot under the 2026-07 syntax:
  fields are **already non-null by default**.  `const x: integer` is non-null +
  write-once; `const x: integer?` is nullable + write-once (may stay null forever).
  Keep them orthogonal — no coupling.
- **`pub const field`?**  Struct fields have no visibility modifiers today (`pub`
  on a field is consumed and ignored, `definitions.rs:2462`).  Out of scope.
- **Const on enum-variant fields?**  In scope — step 3 covers the variant loop
  alongside the struct loop.

## Test strategy

Positive shapes (run + assert), negative shapes (`@EXPECT_ERROR`), plus the real
consumer from step 7.  Current syntax (no `not null`):

```loft
// p386_const_field_constructed_and_read
struct Token { const id: integer }
t = Token { id: 42 };
assert(t.id == 42, "read after construction");

// p386_const_field_with_default
struct Cfg { const port: integer = 8080 }
c1 = Cfg {};
c2 = Cfg { port: 9000 };
assert(c1.port == 8080 && c2.port == 9000, "default + override");
```

```loft
// p386_const_field_reassign_rejected   @EXPECT_ERROR
struct Token { const id: integer }
fn main() { t = Token { id: 1 }; t.id = 99; }

// p386_const_field_reassign_via_ref     @EXPECT_ERROR
struct Token { const id: integer }
fn touch(t: &Token) { t.id = 99; }

// p386_const_virtual_rejected           @EXPECT_ERROR
struct Bad { const v: integer virtual($.x * 2), x: integer }
```

## See also

- [INCONSISTENCIES.md § 33](../../INCONSISTENCIES.md#33-const-applies-to-locals-and-parameters-but-not-fields) — the gap this closes
- `@P386` / `src/parser/definitions.rs:2467` — the existing parser guard that is the landing site
- `@F18` — the const-**parameter** feature (`Argument.constant`) whose enforcement shape this mirrors
- [LOFT.md § Field modifiers](../../LOFT.md) — current modifier list (to be extended in step 6)
- `.claude/skills/loft-write/SKILL.md` — user-facing field-modifier reference (step 6)
- A `lib/*` grid/cell consumer with the rebuild-via-construction tick loop — the step-7 dogfood target (pick the current one at build time; the historic pointers `lib/world` and `lib/hex_world` have both moved)
