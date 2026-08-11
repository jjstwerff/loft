# @PLN125 — First-grade library types

**Status:** ACTIVE · **Issue:** [loft-lang/plans#125](https://github.com/loft-lang/plans/issues/125)

The gaps that stop a library type being indistinguishable from a built-in one.
The capability map this plan is measured against lives in
[INTERFACES.md § How first-grade a library type is](../../INTERFACES.md); the
issue carries the argument for each arc. This directory holds only what the
issue cannot: the running record of what shipped, and the instruments.

| arc | what | state |
|---|---|---|
| **A** | associated types — an interface names a companion type | A1 / A2a / A2b / **A2c** shipped; A3–A4 here |
| **B** | a hook at scope end (`#drop`) | not started |
| **C** | `x[i]` dispatched to a library type | not started |

Each arc lands **inert first**: the contract declared, every existing program
proved byte-identical in IR and native Rust, before any new behaviour routes
through it. `bytecode-comparisons/` holds those instruments.

---

## Arc A — where it stands

`type X: B` parses (A1), an implementor's return type is part of satisfaction
(A2a), and `Self.X` in a signature resolves to a placeholder (A2b). What is
missing is the step that makes the placeholder mean anything:

```loft
fn first_width<S: Source>(s: S) -> integer {
  r = s.open();          // r : Source.Rows — a placeholder
  return r.width();      // error: generic type Source.Rows:
}                        //        field access requires a concrete type
```

**A2c's invariant:**

> An associated type is a type variable owned by the interface. Inside a generic
> it dispatches through its declared bounds exactly as `<T: I>` does, and at
> instantiation it binds to the one concrete type the implementor's methods
> agree on — which must satisfy those bounds.

Phrased that way A2c is not new machinery: it is the bounded-type-variable
machinery applied to a second holder. A monomorph already substitutes one
`(type variable → concrete)` pair through the template's code, variables,
attributes and return type; A2c makes that a LIST and appends one pair per
associated type.

**A2c is SHIPPED.** Three parts, each the second user of something that already
existed:

| part | where | reuses |
|---|---|---|
| the template can CALL the companion's methods | `create_bound_method_stubs` now builds stubs for an associated type too | the `t_<LEN><Holder>_<method>` stubs a bounded `<T: I>` already gets |
| the monomorph BINDS it | `associated_bindings` + a list of `(holder → concrete)` pairs in `instantiate_generic` | `substitute_type` / `substitute_type_in_value` / `Function::substitute_type`, applied once more |
| the declared bound is CHECKED | `check_satisfaction` asks the same structural question of the companion | `satisfaction_failures`, extracted so both askers word their own message |

The companion is inferred from the implementor's own signature, read back through
the interface's — where the interface writes `Self.Rows`, the implementor writes a
concrete type, and that is the companion. Nothing else can say it: satisfaction is
structural, so there is no `impl` block to name it in. Return AND parameter
positions are read and must agree; disagreement is refused rather than resolved by
declaration order.

**The gate it was built against:** the monomorph of `bound_width<S: Source>` is
byte-identical to `hand_width(s: FileSource)` written out by hand — same locals,
same retbuf, same frees, same order. Only the live-dispatch preamble differs, and
no monomorph has ever carried one.

Two refusals ship with it, both in `tests/parse_errors.rs`:

```
'S1' binds 'Source.Rows' to 'Bad', which does not satisfy the declared bound 'Cursor': missing width
'S2' does not agree with itself about 'Source.Rows': 'open' and 'feed' name different types for it
```

The first names all four parties on purpose — three of them are invisible at the
call site, where the reader only wrote `use_it(s)`.

An associated type's name is now enforced CamelCase like every other type name.
It stopped being cosmetic when it became the `t_<LEN><Interface>.<Name>_<method>`
stub name whose LEN prefix is parsed back to recover the method: an underscore in
it would split that name in the wrong place and leave the call on the template
stub, which native emits as `todo!()`.

## What the instrument found first

Building the before/after corpus for A2c surfaced three defects in the code path
arc A depends on — all of them already on `main`, none of them filed, because
the whole point of arc A is a method that RETURNS a companion struct and that is
the path they sit in. Written down before any of them was worked:

| # | shape | symptom | on A2c's path? |
|---|---|---|---|
| **1** | an interface method returning a struct/vector/enum **declared later in the file** | `Incorrect var __ref_2[65535]` — an internal compiler error, BOTH backends | **yes** — `Self.X` is a forward reference by construction |
| **2** | a bounded-generic **operator** returning `Self`, consumed inline (`assert(op_of(i,j).w == 10)`) | one store per call not freed | no — a method-shaped return is clean |
| **3** | an interface's associated-type placeholder | minted as a METHOD stub (`t_1D_SqlDb.Rows`), which native emits as `todo!()` | no — dead weight, not wrong output |

**#1 is the blocker.** A bound-method stub stands in for the concrete method
`re_resolve_call` will substitute, so its ABI must equal that method's ABI — and
the hidden `__retbuf` parameter that a struct/vector/enum return carries was
decided on the FIRST pass, from a return type that was still an unresolved
forward reference, and never revisited (the second pass sees the stub already
exists and skips it). The monomorph then gained a `__ref_N` with no scope and no
initialiser, so it never got a stack slot.

The fix is at the one place that enforces that invariant: the second pass
REFRESHES an existing stub's signature instead of skipping it. Everywhere the
two passes already agree the refresh is idempotent, which is what
`bytecode-comparisons/bound-stubs-corpus.loft` proves.

**#3 is a one-line sibling.** `check_satisfaction` already tells an interface's
method stubs from its associated-type placeholders by `DefType` — the stub
builder walks the same children and did not.

**#2 is left open** — it is a leak, not a corruption, it is narrower than it
first looked (a method-shaped return, a bound local, and the non-generic
operator are all clean), and it is not what arc A needs. Recorded here rather
than fixed inside another change.

---

## Instruments

| file | what it is for |
|---|---|
| `bytecode-comparisons/bound-stubs-corpus.loft` | one generic per stub-ABI branch (scalar / text / struct / vector / operator / shared method name). The before/after diff must be EMPTY for any change that only reshapes the stub builder. |
| `bytecode-comparisons/a2c-companion.loft` | the hand-monomorphised reference — the IR A2c's monomorph must equal. |
| `bytecode-comparisons/a2c-companion-bound.loft` | the same body written against the interface. The diff between the two is A2c's specification. |
