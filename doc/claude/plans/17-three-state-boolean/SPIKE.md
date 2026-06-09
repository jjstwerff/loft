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

## Sub-arc B/C/E implementation attempt (decision A, 2026-06-09)

Full diff preserved as [`impl-attempt.diff`](impl-attempt.diff).  **Interpreter:
DONE + verified** (design A — raw `==`, truthiness coerce, `== null` falls out).
**Native: core proven, not complete** — reverted to keep the branch green (shared
codegen forbids a half-done commit).

What is DONE and proven on the interpreter (the `impl-attempt.diff` interp half):
- `OpConvBoolFromNull` producer; `create.rs` Boolean arm reads operand as `u8`;
  `OpVarBool` reads `u8`; `OpGotoFalse`/`OpNot` coerce (`!= 1`); `eq_bool`/`ne_bool`
  stay **raw** (design A → `null==false` false, `null==null` true); `OpCastTextFromBool`
  renders `"null"`; `null(Boolean)` emits the producer.  Full `{false,true,null}` ×
  `{==false,==true,==null,if,!,fmt}` matrix green on `--interpret`, incl. the
  `hash → boolean` consumer.

What is PROVEN for native (compiles + runs for the core):
- `generation/mod.rs::rust_type`: `Boolean` → `u8` in Variable/Argument/Result/Reference,
  else `bool` (the two-form split — mirrors text String/Str).
- `calls.rs` template substitution wraps boolean operands `((expr) as u8)` (idempotent
  for u8, 0/1 for bool) — fixes the op-template seam wholesale.
- `narrow_int_cast` gains `Boolean => Some("u8")` — central `bool→u8` coercion at the
  return / store / arg seams.
- `output_test_predicate` coerces a boolean `if`/`while` test `((test) as u8) == 1`
  (uniform for both u8 and bool tests).
- Result: local vars, `!`, `==`, `== null`, `if`, field storage, and the
  `hash → boolean` consumer **compile and run correctly on `--native`**.

Remaining native seams (the routed work — ~19 stdlib errors, three patterns):
1. **`&&` / `||` lowering** — native lowers these to a nested if-value expression
   (`if a {true} else {b}`); with boolean arms now mixed `bool`/`u8` the arms
   disagree (`if`/`else` incompatible types).  Needs the logical-lowering arms
   normalised to one form.
2. **`bool`/`u8` → `i64` promotion** — a native "scalar value" path defaults boolean
   into `i64` in some positions (vector element / value emit), giving
   `expected u8, found i64`.  Find the scalar-emit default and add a Boolean arm.
3. A few residual `expected u8, found bool` store/arg spots `narrow_int_cast` doesn't
   reach (non-Set assignment forms).

These are concentrated in the **stdlib**'s use of `&&`/`||` over booleans, not the
user program.  Verdict: native E is the M-sized gating piece the plan predicted;
the approach is validated and the seams are localized — it needs a focused session,
not a budget-pressured patch.

## Reproduce

```
# apply spike.diff, then:
make fill && cargo build --bin loft
target/debug/loft --interpret <spike.loft>   # interpreter: works
target/debug/loft <spike.loft>               # native: 56 type errors (the finding)
```
