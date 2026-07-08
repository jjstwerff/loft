<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN99 — First-grade custom types: direct operator dispatch + custom-format hook

> **Subject:** loft   ·   **Type:** plan   ·   **Area:** parser · types · formatting
> **Effort:** M (two focused core changes; the hard dispatch machinery already exists)
> **Value:** **U** (user-facing capability — library types that behave like built-ins) +
> **F** (foundation — every wrapper type: DateTime, money, colour, `Decimal`, URL)
> **Depends-on:** — (consumes the existing I8 operator-interface + bounded-generic machinery)
> **Driven-by:** @PLN8's DateTime tail (`lib_plans/21-datetime`); the 2026-08
> "better PHP / more capable libraries" cycle ([BROADENING.md](../../BROADENING.md))
> **Live status · lifecycle:** [loft-lang/plans ▸ @PLN99](https://github.com/loft-lang/plans/issues/99) ← single source of truth

## Status

**`status:future` — filed 2026-07-08.** Grounded in a probe session (below): the
generics/interface investment already dispatches user operators inside bounded
generics; the two remaining gaps are what keep a library struct from being
*indistinguishable from a built-in in direct use*.

**Probe evidence (2026-07-08, `--interpret` + `--native`, both agree):**
- `dt + 5` on `struct DateTime { ms: integer }` → **correctly rejected** ("No
  matching operator '+' on 'DateTime' and 'integer'"). Distinct-type safety is
  **free** — a struct is already a distinct nominal `Type::Reference`.
- `smaller<T: Ordered>(a, b) { a < b }` over that struct → **`true`**. User
  `fn OpLt(self, other)` **dispatches inside a bounded generic**.
- The *same* `OpLt`, used **directly** as `a < b` → **"No matching operator '<'
  on 'S' and 'S'"**. Direct concrete operator dispatch is the gap (Arc A).
- `{d.format("date")}` (a library method in interpolation) → **`2026-07-08`**;
  `{d}` (bare) → the generic dump `{ms:…}`. Custom formatting *via an explicit
  method* works; the `{d:date}` **sugar** does not (Arc B).

## Goal

A user library **struct behaves exactly like a built-in type**: chronological /
numeric operators work in *direct* expressions (not only inside `<T: …>`), and
`{x:spec}` renders through the type's own formatter. No new value category, no
per-type built-in — the *general* capability that makes DateTime, money, colour,
a DB `Decimal`, and a URL all first-grade with one investment.

## Effort + design

- **Effort:** M. Both changes are *wiring into machinery that already exists*
  (operator resolution for generics; the `to_text` path for bounded generics) —
  not building dispatch or formatting from scratch.
- **Design north-star:** the direct path should reuse the *same* user-operator
  lookup the bounded-generic path performs (`get_possible` / `call_op`), and the
  format hook is the loft analog of Python `__format__(self, spec)` — the type
  owns its spec vocabulary; core learns nothing of date/money tokens.

## Sub-arcs

### Arc A — direct concrete operator dispatch *(the one load-bearing core change)*
`a < b` / `a == b` / `a - b` on two concrete user-struct operands must resolve
the user `fn Op<Name>(self, other)` def — the resolution the generic path already
does via `get_possible`. Today the direct binary-op path does not consult user
operator defs for concrete `Type::Reference` operands (probe A above). Wire it to
the same lookup; a struct with no such def errors exactly as today (nothing
regresses). **Falsify first (design-protocol):** confirm the user-operator def is
discoverable in *both* parser passes at a direct call site (the bounded-generic
path relies on early signature collection — prove it holds for concrete structs,
or pass 1 and pass 2 disagree on whether `<` is a user op and the token stream
desyncs). Cite `src/parser/mod.rs:3964` (`call_op`), `INTERFACES.md` I8.

### Arc B — the `{x:spec}` custom-format hook
Generalise `try_bound_to_text_call` (`src/parser/collections.rs:1037`) +
`append_data`'s `Type::Reference` arm (`:1166`): **(1)** drop the generic-only
gate — try `t_<len><Type>_to_text` for *any* `Type::Reference(d_nr, _)`, not just
the current generic's type variable; **(2)** thread the raw spec text as a `text`
argument (`""` for bare `{x}`). Then branch the spec-parse
(`src/parser/objects.rs:1367-1406`) on the value type `tp` (known before `:` is
consumed): built-in → today's numeric grammar; custom struct with `to_text` →
read the spec as a **free-form raw string up to `}`** and hand it over. Full
prior design: [`lib_plans/21-datetime/DESIGN.md`](../../lib_plans/21-datetime/DESIGN.md)
Part 1 + Part 2. A struct with no `to_text` renders exactly as today.

### Arc C — inline / value structs *(deferred — perf only, trigger-gated)*
Small structs stored **by value** (inline), not by `DbRef` + heap record +
`OpFreeRef` lifetime — the cost that could bite a hot path (a DB timestamp column
should not heap-alloc a `DateTime` per row). **Trigger:** profiling shows the
per-value cost matters in a real hot loop. Mitigated meanwhile by *lazy
materialization* (a DB client keeps the raw `i64` in its row buffer, materialises
the struct only when the cell is read). Benefits every small wrapper type, not
DateTime alone.

## Composition matrix — Stage A (before Arc A code)

Vary ONE axis per probe, hand-compute each cell, run both backends:
- **operator** × {`<`, `<=`, `>`, `==`, `!=`, `-`} on a concrete struct — which
  already work (default struct `==`?) vs need Arc A. (Probe found `==` did *not*
  error while `<` did — pin whether that is default struct-equality or a real
  binding, so Arc A neither under- nor over-reaches.)
- **context** × {direct expr, `<T: Ordered>` body, `sorted<T[k]>` insert, `if a<b`}.
- **operand shape** × {two locals, literal-vs-local, field-of-struct, call result}.
- **backend** × {interpret, native} — cross-mode divergence is real for dispatch.

## Open design questions

1. **Default struct `==`** — does the probe's non-erroring `==` mean loft already
   gives structs field-wise equality? If so, Arc A is `<`/`<=`/`>`/`>=`/`-`, and
   `==`/`!=` may already be first-grade. Settle in Stage A.
2. **`-` operator name** — the 21-datetime DESIGN named it `OpMin`; the real
   subtraction op name must be confirmed (`OpMin` reads as *minimum*). Nail the
   `Op<Name>` ↔ symbol table before Arc A.
3. **Width/align on custom `{x:spec}`** — `{dt:date>12}`. v1: the type owns the
   whole spec (no outer padding). v2 (only if it earns its keep): layer generic
   align+width over the `to_text` result.
4. **A home for the format hook** — it is a general language capability, not a
   `time` detail; this plan is that home.

## See also

- [`lib_plans/21-datetime/DESIGN.md`](../../lib_plans/21-datetime/DESIGN.md) — the
  format-hook design (Part 1+2) + the DateTime struct that consumes this; **correct
  its claim** that direct-use operators need no core work (they do — Arc A).
- [`@PLN8`](https://github.com/loft-lang/plans/issues/8) — the DateTime tail this unblocks.
- [INTERFACES.md](../../INTERFACES.md) — I8 operator interfaces / `Ordered`, the
  machinery Arc A extends to the direct path.
- [BROADENING.md](../../BROADENING.md) — the 2026-08 "more capable libraries" cycle.
- `src/parser/collections.rs:1037` / `:1166`, `src/parser/objects.rs:1367`,
  `src/parser/mod.rs:3964` — the touch points.
