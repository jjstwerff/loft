<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Enhancement — expected-type propagation into struct-literal field values

**Status: design, not started.**  Spun off from @PLN40's step-7 dogfood (see the
README § Harvested lesson).  Should become its own plan / `loft-lang/features`
issue before build — it is a language-inference change, independent of `const`.

## The gap

An assignment pushes the target field's declared type into its RHS as the
**expected type**, so a width-driven read sizes itself correctly:

```loft
cell.c_color = f#read;        // c_color is u8 → reads 1 byte
```

The equivalent struct-literal field does **not**, so the same expression falls
back to its context-free default (`f#read` → text) and mis-parses:

```loft
Cell { c_color: f#read }      // ERROR: cannot assign text to field of type integer(0,255)
```

Today this is papered over by writing the widths explicitly
(`Cell { c_color: f#read as u8 }`).  `const` fields make it bite, because they
remove the LHS-driven escape hatch (`cell.c_color = …` is now a compile error), so
construction MUST use the literal form — that is how the dogfood surfaced it.

## Root cause (two sites, one missing line)

The parser carries a single "expected type" context, `self.expected`, that
type-driven parses (a bare `f#read`, an untyped `null`, an integer literal's width,
lambda parameter inference, hash/collection element typing) consult.

- **Assignment RHS** — `src/parser/expressions.rs:1228` sets it before parsing:
  ```rust
  let prev = std::mem::replace(&mut self.expected, f_type.clone());
  let s_type = self.parse_operators(f_type, code, &mut parent_tp, 0);
  ```
- **Struct-literal field value** — `src/parser/objects.rs:2195` passes the field
  type to `parse_operators` but never sets `self.expected`:
  ```rust
  let exp_tp = self.parse_operators(&td, &mut value, &mut parent_tp, 0);
  //           ^ td is passed, but self.expected is NOT set to td
  ```
- A **field default** value already sets it (`src/parser/definitions.rs:2832`:
  `self.expected = a_type.clone()`), so the literal path is the lone omission
  among the three construction-ish contexts.

## The invariant

> A struct-literal field value parses under the SAME expected-type context as the
> equivalent assignment RHS — the field's declared type — so `T{ f: expr }` and
> `t.f = expr` infer `expr` identically.

## The candidate fix (small diff, broad reach)

Set `self.expected = td` (save/restore) around the `parse_operators(&td, …)` at
`objects.rs:2195`, mirroring the assignment path.  ~4 lines.

**Why it is nonetheless complex:** `self.expected` is read/written at ~26 sites
across `parser/{mod,control,expressions,definitions,fields,vectors}.rs`.  Setting
it for every struct-literal field changes inference for EVERY struct literal in all
user code AND the stdlib (thousands of literals) — not just `f#read`, but untyped
`null` → typed null, integer-literal width coercion, lambda inference, and
collection/hash element typing.  Most of those should already agree with the
passed `td` argument, but "should" is the risk: any cell that silently changes is a
behaviour change to prove, not assume.

## Safe small steps

Each step is independently verifiable; the risk (a stdlib-wide inference shift) is
isolated behind a gate until proven.

| # | Step | Verify | Why safe |
|---|---|---|---|
| 0 | **Baseline matrix.**  One `/tmp` probe per value-kind (`f#read`, bare int literal into a narrow field, untyped `null`, nested `S{…}` literal, lambda, enum variant, `[]`/hash literal), each in BOTH a `t.f = expr` assignment AND a `T{ f: expr }` literal.  Record what each produces today on both backends. | The matrix shows which cells already AGREE (assignment == literal) and which DIVERGE (`f#read` known; find any others). | No code change |
| 1 | **Gate the change.**  Add `self.expected = td` (save/restore) at `objects.rs:2195`, behind an env flag (`LOFT_LITERAL_EXPECTED`, default OFF). | Builds; suite green with the gate OFF (inert). | Gated → zero default behaviour change |
| 2 | **A/B the matrix under the gate.**  Re-run step-0 probes with the gate ON, both backends.  REQUIRE: the `f#read` cell flips to width-inferred (matches assignment); every already-agreeing cell is UNCHANGED. | Matrix: `f#read` fixed, no other cell regresses. | The gate makes the change observable without committing to it |
| 3 | **A/B the full suite + stdlib parse.**  Run the whole suite gate-ON vs gate-OFF (the stdlib is the real blast-radius test — thousands of literals).  Diff the failures.  Any gate-ON-only failure is a real inference shift to understand and fix (narrow the change, or fix the newly-exposed site). | Gate-ON suite == gate-OFF suite (both backends). | The gate isolates the blast radius; a red cell names the exact site |
| 4 | **Flip default-on.**  Make the propagation the default (opt-out `LOFT_NO_LITERAL_EXPECTED`), soak the suite, then delete the gate + the opt-out. | Full `make ci` green, both backends. | Only after step 3 proves the stdlib is unaffected |
| 5 | **Close the loop.**  Drop the explicit `f#read as u8`/`as u16` from `hex_world.world_load` (now inferred) — the harvested lesson is resolved. | hex_world 16/16 tests green, both backends. | Consumer proof the enhancement did its job |

## Risks / open questions

- **Narrowing vs breadth.**  If step 3 shows the broad `self.expected = td` shifts
  cells beyond `f#read` (e.g. changes an integer-literal coercion in the stdlib),
  prefer NARROWING: propagate the expected type only for the value-kinds that
  need it (a context-free read), rather than unconditionally.  The matrix decides.
- **Nested literals.**  `T{ inner: S{ … } }` already recurses through
  `parse_object`; confirm the save/restore nests correctly (each field restores the
  outer `self.expected`).
- **Interaction with the collection-field priming** at `objects.rs:2161` (the
  in-field write-through target) — the gate must sit AFTER priming so it doesn't
  disturb the collection path.

## See also

- [README.md § Harvested lesson](README.md) — where this surfaced (the hex_world dogfood)
- `src/parser/expressions.rs:1228` — the assignment path that already does this
- `src/parser/objects.rs:2195` — the literal path that does not
- the `engineering-rigor` / `loft-codegen` skills — the matrix-first + prove-both-backends discipline this plan follows
