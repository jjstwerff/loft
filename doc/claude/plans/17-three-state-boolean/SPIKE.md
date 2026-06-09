<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN17 Sub-arc B/C spike — the 255 representation (2026-06-09)

**Throwaway spike. Code reverted to keep the branch green; the diff is preserved as
[`spike.diff`](spike.diff) and the learning is below.**  Per the Design-Protocol
"build is the last probe": the invariant held under construction, and the build
surfaced one axis desk-reasoning missed (native representation).

## What was built (interpreter only)

Five edits, all small (see `spike.diff`):

1. **Producer** — new op `OpConvBoolFromNull() -> boolean; #rust"255u8"` in
   `default/01_code.loft`.  Opcodes are declared in the stdlib and `fill.rs` is
   regenerated (`make fill`) — the "table is full" worry was wrong; the table grows.
2. **Generator** — a `Type::Boolean` arm in `src/create.rs` reads a boolean operand
   as raw `u8` (mirroring the existing `char`-UB precedent), because
   `*get_stack::<bool>()` on byte `255` is **undefined behaviour**.
3. **Truthiness ops** — `OpGotoFalse`/`OpGotoFalseWord`/`OpNot` templates changed
   from `!@v` to `@v != 1` (true is byte 1; both `0` and `255` are falsy → coerce).
4. **Value-movement / compare ops** — `OpVarBool` reads `::<u8>`; `OpPutBool`,
   `OpEqBool`, `OpNeBool` now operate on the raw byte (preserve `255`,
   distinguish it).
5. **Producer wiring** — `parser/mod.rs::null(Type::Boolean)` emits
   `OpConvBoolFromNull` instead of the #256 rejection.

## Result — invariant CONFIRMED on `--interpret`

`spike.loft` distinguishing signature (`==false` / `==true` / `if` / `!` / `{fmt}`):

| value | ==false | ==true | if | ! | fmt |
|---|---|---|---|---|---|
| real false | true | false | else | 1 | false |
| real true | false | true | then | 0 | true |
| **null** | **false** | **false** | else | 1 | **null** |
| hash present-false | true | false | else | 1 | false |
| hash present-true | false | true | then | 0 | true |
| **hash ABSENT** | **false** | **false** | else | 1 | **null** |

The invariant ("`255` preserved by storage / compare; coerced to false at the
truthiness chokepoint") held with the **bounded** 7-op + generator change predicted
in the README — no per-shape code.  The motivating `hash → boolean` accessor
compiles and ABSENT is observably distinct from present-false.  Note: `--interpret`
was required — the default CLI path is native (next finding).

## The finding desk-reasoning missed — native is a representation change, not a marshal tweak

The `#rust` templates are **shared** between interpreter and native, but the
**operand type differs per backend**: the interpreter stores booleans as **bytes**
(so `255` fits and the generator change makes reads `u8`), while native lowers
booleans to **Rust `bool`** locals.  So the `@v != 1` template, correct for a `u8`
in the interpreter, becomes `bool != integer` in native — **56 native codegen type
errors** across the whole stdlib (`if (n_exists(...)) != 1`, etc.).

This re-sizes **sub-arc E**: native tri-state boolean is **not** "coerce at the
`#rust` `bool` marshal boundary" — it requires native's boolean representation to
become `u8` end-to-end (store reads, locals, comparisons), in lockstep with the
template change, or the templates must branch per backend.  This is a Goal-D parity
hazard: the two backends disagree on the boolean's *type*, not just its value.  It
is M-sized on its own and is the gating risk for the whole feature.

## Implications for phasing

- B (representation) + C (truthiness chokepoint) are **proven cheap on the
  interpreter** — the bounded op set is correct.
- The shared-template architecture means a template can't assume `u8` until **both**
  backends supply `u8`.  So sub-arc E (native rep → `u8`) must land *with or before*
  the template change, not after — they cannot be separated without breaking native.
- New op `OpConvBoolFromNull` is the clean producer (no opcode-budget problem).
- The #256 guard cluster (`null`/`return null`, `??`, `== null`) is what flips to
  *support*; the spike only flipped the `null`-literal guard — `??` and `== null`
  remain (sub-arc G256 / D).

## Reproduce

```
# apply spike.diff, then:
make fill && cargo build --bin loft
target/debug/loft --interpret <spike.loft>   # interpreter: works
target/debug/loft <spike.loft>               # native: 56 type errors (the finding)
```
