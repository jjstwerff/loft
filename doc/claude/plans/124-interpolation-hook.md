<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 124 — a library-implementable interpolation

## Status

**@PLN124 H1–H5 built.** A format string whose TARGET TYPE implements the
interpolation contract hands over its literal and hole parts instead of appending
them into a text buffer. `text` is unchanged, proven as a byte-identical IR diff.
Its first consumer is @PLN23 S4, which is built on it.

The design — why neither the type system nor `const` can carry the distinction,
and why the parser already knows it — is
[@PLN23's INTERPOLATION_HOOK.md](23-db-clients/INTERPOLATION_HOOK.md). This
file records what was BUILT and what it cost.

## The contract

A struct satisfies it structurally, the way every loft interface works:

```loft
fn lit(self: T, s: text)              // a literal chunk the AUTHOR wrote
fn hole_text(self: T, v: text?)       // an interpolated VALUE
fn hole_int(self: T, v: integer)
fn hole_float(self: T, v: float)
fn hole_single(self: T, v: single)
fn hole_boolean(self: T, v: boolean)
fn hole_character(self: T, v: character)
```

`lit` is the whole test for whether the hook applies: a type that can accept the
author's literal bytes is a type that can be built. The `hole_*` methods it goes
on to define say which value kinds it takes, and **a kind it does not define is a
compile error naming the method to add** — never a quiet fall back to text, which
would put a value back on the path this exists to close.

```loft
q: SqlText = "SELECT id FROM t WHERE name = {name}";
```

lowers to `q.lit("SELECT id FROM t WHERE name = "); q.hole_text(name);` with the
accumulator itself as the value of the expression.

## Where the target comes from

`Parser::interpolation_target` is a fifth SHAPE read off the one `⇐` expected-type
channel, beside `lambda_hint` / `enum_hint` / `vector_hint` / `read_target_type` —
not a sixth side-channel. `var_tp` carries the target for a typed-local
declaration, a typed reassignment and a struct-field init; `expected` carries it
for a call argument (free function and method) and a return body. That is the
same two-source rule the bare-enum-variant branch already used.

## What it cost, and what the build corrected

**The mint has to be pass-stable, and that is the sharp edge.** Taking the branch
mints an accumulator variable, and the variable tables persist across passes BY
NAME (loft#662), so a decision that differed between passes would shift every
later work variable underneath itself. Method defs are collected on both passes —
the property the `to_text` hook already relies on — so the branch is stable; and
the accumulator draws from its own `__fmt_N` counter (`Function::work_format`)
rather than sharing `__work_N` or `__ref_N`, for the same reason every other work
namespace has one of its own.

**The expected-type channel leaked across a nested call.** The hints that read it
each SET it when they applied and none CLEARED it when they did not, so in
`take(build_one("arg"))` the string literal — `build_one`'s `text` parameter — was
checked against `take`'s parameter type. Latent before this arc (the enum,
collection and function shapes rarely nest that way) and immediate once a `text`
parameter could be shadowed by an outer struct target. The channel is now cleared
per argument, at both the free-function and the method site.

**A format spec on a hole is REFUSED.** `"{x:>8}"` into a built type has nothing
to format — the value is handed over, not rendered — so it is an error rather than
a silently ignored spec.

**Nullability is not a kind.** A `text?` hole is a text hole whose value may be
absent; `format_hole` peels `Optional` and lets the target's own `hole_text`
parameter type decide whether it accepts one. That is what lets `SqlText` make SQL
NULL a distinct bound value rather than the text `"null"`.

## Proof

- **H1, inertness.** `124-interpolation-hook/bytecode-comparisons/format-corpus.loft` is one function per
  format-string path the dispatch can reach — literals, bare holes, alternation,
  the numeric spec grammar, a `text?` hole, a fault-prone hole (`OpTagFault`), an
  inner fault that must NOT tag, struct/JSON/pretty specs, a custom `to_text` spec,
  expression holes, three `for`-comprehension forms, backtick multiline, escaped
  braces, `+=` accumulation, and argument position. 104 format sites.
  `loft introspect` before/after is **byte-identical**; an empty diff is the whole
  proof, and it is re-checked after each change to the arc.
- **H2/H3/H4.** `tests/scripts/interpolation-hook.loft` asserts the call SEQUENCE
  rather than the result — a target that only checked the final string could not
  tell the hook from ordinary formatting. Every scalar kind routes to its own
  method; both backends.
- **The target shape was captured BEFORE the parser was touched.**
  `124-interpolation-hook/bytecode-comparisons/target-shape.loft` is the hand-written program whose IR
  the branch had to reproduce, proven on both backends first. It also settled a
  design question by measurement: a default-constructed `T { }` is equivalent to a
  named constructor, so the contract needs only methods and `Interpolated` stays a
  pure interface.

## What is NOT built

- **`SqlIdent`** (H6) — the deliberate exception, where an identifier is validated
  and quoted INLINE. Nothing is built; today every hole is a value.
- **Procedures** (H7) — needs H6 first.
- **A boxed value type** collapsing the per-kind `hole_*` methods into one
  (@PLN125 arc A, associated types). The per-kind form was chosen first precisely
  because it can be collapsed later without changing what the author writes.
- **Specs on holes** are refused rather than delivered; if a target ever wants
  them, they have to reach the hole as data, not as a rendering.
