<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 21 — DateTime: a library struct + one general format hook (not a built-in type)

Code-grounded design for the remaining tail of [`@PLN8`](README.md). The basics
(arc D — the pure-loft `time` library over `integer` epoch-ms) shipped as
`time 0.1.0`. This document **supersedes** the earlier "build a distinct built-in
`Type::DateTime`" plan (arcs A/B/C): an evaluation against the present code
(2026-06-14) showed a **library struct** `DateTime { ms: integer }` gets every
property the built-in was for — at a fraction of the cost, and with the missing
piece (custom formatting) generalised into a feature **every** library reuses.

> **Status: DEFERRED past the 2026-07 release.** This is post-release work and
> fits the 2026-08 "better PHP / more capable libraries" cycle — the one core
> change here (a per-type format hook) is exactly the "more complex
> libraries/types" capability that cycle is about. No code lands now.

---

## The decision (one sentence)

> `DateTime` becomes a **`time`-library struct** `DateTime { ms: integer }` — a
> distinct nominal type that **cannot** be confused with `integer`, with
> chronological operators defined as ordinary library operator functions and
> date formatting served by **one new general core feature**: `append_data`
> dispatches `{x}` / `{x:spec}` on a user struct to that struct's own
> `to_text(self, spec)` method. **No built-in `Type::DateTime`, no core
> conversion opcodes, no core format opcodes.**

## Why the struct wins — evaluated against current code (2026-06-14)

The built-in's whole purpose was a *distinct type that shares integer storage but
not integer behavior*. A struct is **already** a distinct type, so three of the
four properties come for free; the fourth (formatting) is the only gap, and it is
better filled by a general hook than by a one-off built-in.

| Property the built-in was for | A struct gives it via | Core change? |
|---|---|---|
| `dt + 5` is a compile error | `DateTime` is `Type::Reference`; `can_convert` never coerces a reference to an `integer` param (`parser/mod.rs:1517`), so no `Op*Int` accepts it, and no `OpAdd(DateTime, …)` is defined. **Impossible by construction.** | none |
| `dt1 < dt2`, `dt1 == dt2`, `dt2 - dt1` | library operator defs — `call_op` looks up *all* `OpLt`/`OpEq`/`OpMin` defs via `get_possible` (`parser/mod.rs:3964`); user types dispatch exactly like primitives (`default/01_code.loft:42` documents "any user type defining OpLt" satisfies `Ordered`). | none |
| civil-calendar math (`epoch_ms ↔ y/mo/d/h/…`) | pure-loft, already shipped in `time 0.1.0` | none |
| `{dt}` / `{dt:date}` formatting | **the one gap** — `append_data` renders a plain struct as a generic dump; custom `to_text` fires only inside bounded generics (`collections.rs:1037`). → close it with the general hook below. | **one feature** |

This **deletes the entire built-in plan**: arc A (the ~25-file `Type::DateTime`
blast radius), arc B (native/wasm core conversion opcodes), and arc C (core
`{dt:…}` format opcodes) all dissolve. What replaces them is a single, reusable
core feature plus pure-library work.

> **Correction (2026-07-08, @PLN99).** The `dt1 < dt2` row above (core change:
> *none*) is **wrong for DIRECT use**. A probe shows user `OpLt` dispatches inside
> a bounded generic (`smaller<T: Ordered>(a,b){a<b}` → `true`) but **not** in a
> direct `a < b` on concrete structs ("No matching operator '<' on 'S' and 'S'").
> So a first-grade `DateTime` needs a **second** core change — *direct concrete
> operator dispatch* — tracked together with the format hook under
> [@PLN99 — first-grade custom types](../../plans/99-first-grade-types/README.md).
> The format hook below stands; distinct-type safety (`dt + 5` rejects) and
> generic-context operators are genuinely free.

## The one core change — generalise the format hook

The feature has **two parts**: the *hook* (how a type declares formatting) and
the *spec parse* (how `{x:spec}` is tokenised for a custom type). The second is
the real work — the current spec grammar actively rejects custom specs.

### Part 1 — the hook: `to_text(self, spec: text) -> text`

`try_bound_to_text_call` (`src/parser/collections.rs:1037`) already lets a value
render through a `to_text` method, and `append_data`'s `Type::Reference` arm
(`:1166`) already routes to it — but **only** when the value's type is the
type-variable of the *current bounded-generic* function (`None` when `context ==
u32::MAX` or the context isn't `DefType::Generic`), and it passes the value + a
hidden work-text buffer, **not the spec** (the spec lives in `state`, never
threaded in). Two changes:

1. **Drop the generic-only gate** — try the `t_<len><Type>_to_text` lookup for
   *any* `Type::Reference(d_nr, _)`, not just the current generic's type variable.
2. **Thread the spec** — pass the raw spec text as a `text` argument (`""` for
   bare `{x}`). The value owns its spec vocabulary — the Python
   `__format__(self, spec)` model; core learns nothing of date tokens. The hidden
   I9 work-text output buffer carries through unchanged.

A struct with no `to_text` formats exactly as today (the generic `OpFormatDatabase`
dump) — nothing regresses. This is the natural loft analog of
`Display`/`__format__`/`ToString`, reusable by every library (money, colour, a DB
`Decimal`, a URL), not a DateTime one-off.

### Part 2 — the spec parse (`src/parser/objects.rs:1367-1406`)

Today's spec grammar is `:` `[padchar][flags][width-expr][radix-id]`, and
`get_radix` (`:1455`) **errors on any identifier outside `{x,b,o,d,f,e,json}`**
(`:1470`) while a bare word like `date` is swallowed as the pad-char token
(`:1374`). So `{dt:iso}` is a compile error and `{dt:date}` is silently wrong —
custom specs do not fit the grammar at all. The fix: **branch on the value type
`tp`** (already known at `:1355`, before the `:` is consumed):

- built-in type → existing grammar, unchanged;
- custom type (struct defining `to_text`) → read the spec as a **free-form raw
  string up to the closing `}`** and hand it to `to_text`.

Simpler than extending the grammar, and strictly more powerful — each type gets
an arbitrary spec DSL (even strftime-style `{dt:%Y-%m-%d}`).

### The load-bearing claim — probe before coding (design-protocol)

The raw-vs-grammar branch **must be pass-stable**: the `t_<len><Type>_to_text`
def must be discoverable when a function body's format string is parsed in
*both* parser passes, or pass 1 parses the spec with the numeric grammar while
pass 2 reads it raw and the **token stream desyncs**. `try_bound_to_text_call`
already relies on early signature collection; the one thing to falsify first is
that it holds for *concrete* (non-generic) structs too.

### One deferred sub-decision — width/align on custom types

`{dt:date>12}` (pad the rendered date to width 12). v1: the type owns the whole
spec, no outer padding — simplest, fully general. v2 (only if it earns its keep):
layer the generic align+width over the `to_text` result.

## The `time`-side work (pure library, no core)

Once the hook exists, everything else is a `time` library release:

- `struct DateTime { ms: integer }` (null = the struct ref is null, or `ms ==
  i64::MIN` — pick one and test it).
- Operators: `OpLt/OpLe/OpGt/OpGe/OpEq/OpNe(a: DateTime, b: DateTime) -> boolean`
  (plain `i64` compare on `.ms`); `OpMin(a: DateTime, b: DateTime) -> integer`
  for `dt - dt` → milliseconds. **No** `OpAdd` — `dt + 5` stays a compile error;
  stepping is `time::add_days/add_weeks/add_seconds`.
- `to_text(self: DateTime, spec: text) -> text`: `""`/`datetime` →
  `YYYY-MM-DD HH:MM`, `date` → `YYYY-MM-DD`, `time` → `HH:MM`, `iso` →
  `…THH:MM:SSZ`, `wday` → `Mon`. Body calls the civil-math already in the library.
- Constructors return `DateTime`: `now()`, `from_millis`, `from_ymd`, `parse`,
  `today`. Field accessors and `add_*`/`*_between` change `integer` → `DateTime`
  in their signatures — bodies unchanged (they already operate on the ms value).
- Ships as a new `time` minor release; the training app and any consumer move to
  it when ready (the `integer`-based 0.1.0 keeps working until then).

## The one real tradeoff — and its mitigation

A struct value is a 12-byte `DbRef` + a heap record + `OpFreeRef` lifetime
tracking (`data.rs:1677`, `scopes.rs:14`), versus the built-in's inline 8-byte
`i64`. For most date use (parse, store, compare, format) this is negligible. The
case that *could* bite is the high-performance DB path (@PLN23): a result set
with a timestamp column should **not** heap-allocate a `DateTime` per row.

**Mitigation (a library-design rule, not a core feature):** a DB client keeps the
raw `i64` in its row/cell buffer and materialises a `DateTime` struct **lazily**,
only when the program reads that cell. Bulk scans stay `i64`; the struct cost is
per-materialised-value, not per-row. So the struct model does **not** compromise
the "break out of rustc, fast path into databases" goal.

If profiling ever shows the per-value cost matters in a hot loop, the *general*
answer is inline/value structs (small structs stored by value, not by `DbRef`) —
a language feature that would benefit every small wrapper type, tracked
separately if it earns its keep. It is **not** needed for DateTime.

## What this changes vs. the old design / README

| Old plan (built-in) | This design (struct + hook) |
|---|---|
| Arc A: distinct `Type::DateTime`, ~25 files | **gone** — `DateTime` is a library struct |
| Arc B: native + wasm core conversion opcodes | **gone** — civil math is pure-loft library code (shipped) |
| Arc C: core `{dt:…}` format opcodes | **gone** — replaced by the general `to_text` hook |
| README decision #3: "no per-type format hook, so it must be in core" | **reversed** — the per-type hook is exactly the feature we add |
| Effort H (a new language primitive) | one general S–M core feature + a pure-library `time` release |

## Phasing (post-release)

1. **Core: generalise the format hook** — lift `try_bound_to_text_call` to any
   struct with `to_text(self, spec)`; fall back to the generic dump otherwise.
   Test with a throwaway struct that renders `{x:foo}` custom. This is the only
   core change and it stands alone (useful to every library).
2. **`time` release** — add the `DateTime` struct, operators, `to_text`, and the
   constructor/accessor signature changes; cut a new `time` minor.
3. **Consumers** migrate at will (training app, future DB clients).

This feature needs a home of its own once work starts (the format hook is a
general language capability, not a `time` detail) — a small plan or a
`QUALITY.md` row, created when the 2026-08 cycle picks it up.

## See also

- [README.md](README.md) — the @PLN8 plan + the shipped basics (`time 0.1.0`).
- `src/parser/collections.rs:1037` (`try_bound_to_text_call`) + `:1112`
  (`append_data`) — the format path the hook generalises.
- `src/parser/mod.rs:3964` (`call_op`) — why user-struct operators dispatch.
- `doc/claude/INTERFACES.md` — operator overloading on user types.
- `doc/claude/BROADENING.md` — the 2026-08 "better PHP / more capable libraries"
  cycle this lands in.
