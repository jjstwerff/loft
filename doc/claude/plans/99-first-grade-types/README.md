<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN99 — First-grade custom types: direct operator dispatch + custom-format hook

> **Subject:** loft   ·   **Type:** plan   ·   **Area:** parser · types · formatting
> **Effort:** M (three focused core changes; the hard dispatch machinery already exists)
> **Value:** **U** (user-facing capability — library types that behave like built-ins) +
> **F** (foundation — every wrapper type: DateTime, money, colour, `Decimal`, URL)
> **Depends-on:** — (consumes the existing I8 operator-interface + bounded-generic machinery)
> **Driven-by:** @PLN8's DateTime tail (`lib_plans/21-datetime`); the 2026-08
> "better PHP / more capable libraries" cycle ([BROADENING.md](../../BROADENING.md))
> **Live status · lifecycle:** [loft-lang/plans ▸ @PLN99](https://github.com/loft-lang/plans/issues/99) ← single source of truth

## Status

**`status:future` — filed 2026-07-08.** Grounded in a probe session (below): the
generics/interface investment already dispatches user operators inside bounded
generics; the three remaining gaps are what keep a library struct from being
*indistinguishable from a built-in in direct use*.

**Implementation progress (2026-07-08, branch `tuxedo-work`):**
- **Arc A (direct concrete operator dispatch) — DONE.** `a < b`, `b - a`, `a == b` on
  a plain user struct now dispatch its `fn OpLt`/`OpMin`/`OpEq` (regression
  `tests/scripts/511-first-grade-operators.loft`).
- **Arc B Part 1 + 2 (custom `{x}` / `{x:spec}` format hook) — DONE.** A struct's own
  `to_text(self)` / `to_text(self, spec)` drives interpolation for ANY struct, not only
  inside a generic body; built-in `{n:05d}`/`{n:x}` grammar untouched (regression
  `tests/scripts/512-first-grade-format.loft`).
- **Arc C (user conversions `x as T`) — DONE.** `"lit" as T`, `var as T`, `structA as B`
  dispatch a user `fn OpConv<T>From<S>`, leak-free on both backends; no conversion →
  clean `Unknown cast` error (regression `tests/scripts/513-first-grade-conversions.loft`).
  See STEPS.md § Implementation status for the three-edit mechanism.
- **A5 null-equality facet — DONE.** `s == null` on a nullable struct-reference variable
  (`Optional(Reference)`) now lowers to `OpRefIsNull` (was the broken
  `OpEqBool(is_non_null(s), 255_sentinel)` → always false). Gated on `Optional(Reference)`
  so hash-lookup results (bare `Reference`, `rec==0` miss) keep their correct path
  (regression `tests/scripts/514-null-equality-struct-ref.loft`). `??` and `== null` agree.
- **Arc D (value structs) — deferred (perf trigger).**

**@PLN99 is substantively complete:** Arcs A, B (1+2), C, and the A5 null facet are all
DONE; only the perf-gated Arc D remains deferred.

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
- `"2026-07-08" as DateTime` → **`ms=null`, exit 0**. `as` to a custom type
  *silently* yields a null/garbage value — no user conversion hook and **no clean
  reject** (the worst outcome). A first-grade type must let `as` dispatch a
  user conversion, especially `text as T` parse-on-cast (Arc C).
- **Null equality is broken for custom structs.** `s: DT? = null` then
  `s == null` → **`false`** and `s != null` → **`true`** (both wrong) — while
  `integer? == null` / `text? == null` → `true`, and `s ?? d` + `{s}` (renders
  `null`) both correctly see `s` as null. So `??` and `== null` **disagree**. A
  live correctness bug on `main` *and* a first-grade requirement (Arc A null
  facet). No special null *model* is needed — a struct is a reference and the
  @PLN25 null model covers it; the fix is aligning `==`/`!=`-vs-`null` with the
  built-in nullable path. **Reject** the DESIGN's `ms == i64::MIN` in-band
  sentinel — standard reference-null is the uniform answer.

## Goal

A user library **struct behaves exactly like a built-in type**: chronological /
numeric operators work in *direct* expressions (not only inside `<T: …>`),
`{x:spec}` renders through the type's own formatter, and **`x as T` runs a
user-defined conversion** — including `"literal" as T` (parse-on-cast), which
gives a custom type literal-like syntax. No new value category, no per-type
built-in — the *general* capability that makes DateTime, money, colour, a DB
`Decimal`, and a URL all first-grade with one investment.

## Effort + design

- **Effort:** M. Both changes are *wiring into machinery that already exists*
  (operator resolution for generics; the `to_text` path for bounded generics) —
  not building dispatch or formatting from scratch.
- **Design north-star:** the direct path should reuse the *same* user-operator
  lookup the bounded-generic path performs (`get_possible` / `call_op`), and the
  format hook is the loft analog of Python `__format__(self, spec)` — the type
  owns its spec vocabulary; core learns nothing of date/money tokens.

## Sub-arcs

### Arc A — direct concrete operator dispatch *(load-bearing — operators are used everywhere)*
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

**Null facet (a live `main` bug, fold in here):** `s == null` / `s != null` on a
nullable user struct must match built-in nullables — today a null `DT?` wrongly
reports `== null` → `false` while `integer?`/`text?` report `true` and `??` on the
same value correctly coalesces. This is a *distinct* path from
`Op<Name>(self, other)` (compare-against-`null`, not struct-vs-struct), but the
same contract; the struct's nullability stays standard reference-null — **no
per-type `i64::MIN` sentinel** (that would be the special-casing we avoid).

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

### Arc C — user-defined conversions (`x as T`)
`x as T` on a custom target `T` must run a user conversion, mirroring the
built-in `OpConv<To>From<From>` path (`self.convert`, `src/parser/fields.rs`).
Today it **silently mis-casts** (`"…" as DateTime` → `ms=null`, no error) — the
fix is dispatch-a-user-conversion-or-cleanly-reject. **The high-value special
case: `"literal" as T`** — a `text` value (especially a literal) parses into a
custom type, giving custom types a literal syntax: `"2026-07-08" as DateTime`,
`"#ff0000" as Colour`, `"1.5" as Decimal`. Dispatched by the target type `T`
(from the `as T` annotation) + the source type. **Integrate with `as T ?? default`**
(#512, checked-cast-with-fallback) so a fallible parse has a safe form: a failed
`"bad" as DateTime ?? epoch` discharges to the fallback. Open sub-question: the
*declaration* shape — an `OpConv<T>From<S>` fn (matches the built-in naming), or a
friendlier `fn from(s: S) -> T` / `parse` convention dispatched by return type.

### Arc D — inline / value structs *(deferred — perf only, trigger-gated)*
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

- **[STEPS.md](STEPS.md) — the detailed, verifiable implementation steps** (each
  step: change + a probe with hand-computed expected output, both backends).
- [`lib_plans/21-datetime/DESIGN.md`](../../lib_plans/21-datetime/DESIGN.md) — the
  format-hook design (Part 1+2) + the DateTime struct that consumes this; **correct
  its claim** that direct-use operators need no core work (they do — Arc A).
- [`@PLN8`](https://github.com/loft-lang/plans/issues/8) — the DateTime tail this unblocks.
- [INTERFACES.md](../../INTERFACES.md) — I8 operator interfaces / `Ordered`, the
  machinery Arc A extends to the direct path.
- [BROADENING.md](../../BROADENING.md) — the 2026-08 "more capable libraries" cycle.
- `src/parser/collections.rs:1037` / `:1166`, `src/parser/objects.rs:1367`,
  `src/parser/mod.rs:3964` — the touch points.
