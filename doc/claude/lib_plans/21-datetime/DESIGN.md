<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 21 — DateTime full-support tail — implementation design

Code-grounded design for the remaining tail of [`@PLN8`](README.md): a built-in
`DateTime` value type + `{dt:…}` formatting.  The basics (arc D — the pure-loft
`time` library over `integer` epoch-ms) already shipped as `time 0.1.0`; this
covers **arc B** (native/wasm conversion), **arc C** (`{dt:…}` format opcodes),
and **arc A** (the distinct `Type::DateTime`).  Written against the present code
(line refs are anchors, verify on edit) and run through the
[design-protocol](../../../../.claude/skills/design-protocol/SKILL.md): the
invariant is named, its re-assertion sites counted, and the load-bearing claim
falsified before any code.

---

## The invariant (one sentence)

> A `DateTime` is an `i64` epoch-millisecond value with a **distinct static
> type** that **shares integer STORAGE** (8-byte slot, `i64::MIN` null) but
> inherits **no integer BEHAVIOR** — chronological comparison, date formatting,
> and the *absence* of arithmetic are each selected by `Type::DateTime` at the
> type-keyed dispatch points, so an expression no test covered behaves correctly
> because dispatch keys on the *type*, never on the raw `i64` bits.

## The load-bearing claim — probed, holds

The one way this design fails is **silent over-unification**: `dt + 5` (or
`{dt}` rendering a raw number) compiling as integer behavior because a `DateTime`
is "just an i64". Probed against the present code and it **cannot happen**:

- **Surprise from the probe:** there is no `Type::Long`; a 64-bit integer is
  `Type::Integer(IntegerSpec::wide())`, null = `i64::MIN` (`src/data.rs` enum
  ~1080-1130; `IntegerSpec::wide()` ~93; null checks throughout `src/ops.rs`).
  So "reuse the long path" means *share the wide-`Integer` storage rules*, not a
  separate variant.
- **Operator selection** runs through `call_op` → `process_call_args` →
  `convert` / `can_convert` (`src/parser/mod.rs:4173`, `:1177`, `:1454`). An
  argument matches a parameter **only if `convert` coerces it**; `can_convert`
  coerces solely through *explicit per-type rules* (enum→int, text→text,
  refvar, bare collections, function) and returns **`false`** for everything
  else (`mod.rs:1517`).
- **Therefore:** a new `Type::DateTime` with **no coercion rule added** never
  satisfies an `integer` parameter → `OpAddInt(integer, integer)` rejects `dt` →
  **`dt + 5` is a compile error**. Comparison is **opt-in**: we *define*
  `OpEqDateTime(datetime, datetime)` etc., whose `#rust` bodies do a plain `i64`
  compare. Arithmetic is **forbidden by omission**. The dangerous inheritance is
  impossible by construction, not by vigilance.

This is why the design picks a **distinct variant over a tagged `IntegerSpec`**
(open question Q1): a tag would make `DateTime` inherit *all* integer behavior
unless re-checked at every operator site — `N × silence` on the *dangerous*
side. The distinct variant moves the cost to the **safe** side (storage sites),
where it is contained and test-guarded (next section).

## Brittleness: N × silence, counted before coding

Adding a `Type` variant is part loud (exhaustive `match` → compile error, free
correctness), part silent (catch-all `_ =>` → wrong result if `DateTime` is
omitted). Probing every `match` on `Type` in `src/data.rs` gives the **exact**
silent checklist — `N` is small and known:

| Site (≈line) | Catch-all today | DateTime must |
|---|---|---|
| `element_size` (~1665) | `_ => 0` | return **8** (group with wide `Integer`) |
| `element_align` (~1626) | `_ => 1` | return **8** |
| `to_default` (~741) | `_ => Value::Null` | return the **null DateTime** (`i64::MIN`) |
| `content` (~1335) | `_ => Unknown(0)` | return self (leaf scalar) |
| `typedef::fill_database` Integer arm | — | store `DateTime` as the 8-byte int field |
| `heap_dep` / `heap_def_nr` / `depend` | `_ => None`/`{}` | **no change** (correct for a scalar) |

`Type::name` / `Type::show` are **exhaustive** → adding `DateTime` there is a
compile error (free). So `N ≈ 5` silent sites, all enumerated.

**Cure (design-protocol step 2 — make omission loud / collapse N):**
1. At each silent site, put `DateTime` in the **same arm as wide-`Integer`** —
   one storage rule consulted at each site, not five independent restatements.
2. Add **one storage round-trip test**: a struct with a `DateTime` field —
   set / get / null / compare — exercised on every backend, so a missed site
   fails loudly in CI rather than silently corrupting a date. `N × silence` is
   thus small, listed, and guarded by a single test.

---

## Resolved open questions

- **Q1 storage** → **distinct `Type::DateTime`**, backed by `i64`, null
  `i64::MIN`, storage shared with wide-`Integer`. (Rationale above: dangerous
  inheritance impossible by construction.)
- **Q2 literals** → **constructor-only**. No lexer literal; values come from
  `time::parse` / `time::from_millis` / `now()`. The lexer is untouched.
- **Q3 operators** → define `OpEq/OpNe/OpLt/OpLe/OpGt/OpGe DateTime`
  (chronological, `i64` compare). `dt - dt` → `OpMinDateTime(datetime,datetime)
  -> integer` (ms). **No** `dt + integer` or `dt + dt` — bare arithmetic is a
  compile error (no operator + no coercion); `time::add_days/add_weeks/
  add_seconds` wrap explicit stepping.
- **Q6 tokens** → bare `{dt}` = `YYYY-MM-DD HH:MM` (minute precision — the common
  log/display case; ISO is opt-in). `{dt:date}`=`YYYY-MM-DD`,
  `{dt:time}`=`HH:MM`, `{dt:datetime}`=`…:SS`, `{dt:iso}`=`…TZ`, `{dt:wday}`=`Mon`.
- **Q4 weekday (integer), Q5 name (`time`), Q7 parse strictness** — already
  resolved in the shipped basics; unchanged here.

## Implementation map (per arc, from the probes)

### Arc B — conversion core + parity (build first; zero type-system risk)
- `src/ops.rs`: native `days_from_civil` / `civil_from_days` (Hinnant) realising
  the one contract `epoch_ms ↔ {y, mo, d, h, mi, s, wday}` (UTC).
- wasm: same contract via `js_sys::Date` UTC getters (`src/wasm.rs` / the
  generation wasm path) — no calendar math compiled into the wasm binary.
- **Parity test**: one epoch-ms renders identically on interp / native / browser-
  wasm / wasm32-wasip2, riding `tests/html_wasm.rs::wasm_library_suite`.

### Arc C — `{dt:…}` format opcodes (de-risk on plain `integer` as `{ms:date}`)
- `src/parser/objects.rs` `get_radix()` (~1455): parse the date tokens
  (`date`/`time`/`datetime`/`iso`/`wday`) → a format code.
- `src/parser/collections.rs` `append_data()` (~1112): a `Type::DateTime` arm →
  emit `OpFormatDate` (and an `integer` `{ms:date}` path for the de-risk step).
- `src/generation/ops/mod.rs` (~127, ~219): register `OpFormatDate` /
  `OpFormatStackDate`; `src/generation/ops/text_ops.rs` + `src/generation/
  text.rs`: the `format_date` emitter.
- `src/state/text.rs` + `src/fill.rs`: interpreter handlers.
- `src/ops.rs`: `format_datetime(s, epoch_ms, token)` using arc B; `i64::MIN` →
  `"null"` (mirrors `format_long`).

### Arc A — distinct `Type::DateTime` (largest blast radius, last)
- `src/data.rs`: add the variant; arms in `name`/`show` (loud) and the silent
  checklist `element_size`/`element_align`/`to_default`/`content` (grouped with
  wide-`Integer`).
- `src/typedef.rs`: `"datetime"` keyword → `Type::DateTime`; `fill_database`
  lays it out as the 8-byte int field.
- `default/*.loft`: `OpEq/Lt/Le/Gt/Ge/Ne/Min DateTime` operator defs with
  `#rust` `i64` bodies (→ generate `fill.rs` handlers); `now()` / `from_millis`
  return `DateTime`; `append_data`'s default `{dt}` picks the minute renderer.
- `lib/time` (loft-libs-core): migrate `time::*` signatures `integer → DateTime`
  — a **type-only** change (bodies unchanged) → a new `time` minor release.
- the storage round-trip test (the brittleness guard) + the 4-backend gate.

## Phasing (de-risk order)

1. **Arc B** — conversion + parity. Pure functions, no type changes, fully
   testable in isolation.
2. **Arc C on `integer`** — wire the format opcode end-to-end as `{ms:date}`
   with zero type-system risk; proves the opcode plumbing.
3. **Arc A** — add `Type::DateTime` + operators + the silent-site arms + the
   storage test; flip the `{dt}` default and the `time::*` signatures. Largest
   blast, done last, guarded by the round-trip test + the 4-backend gate.

## Testing

The existing 4-backend gate (interp `wrap.rs` / native `native.rs` / browser +
wasm32-wasip2 `html_wasm.rs`) already exercises `time`; arc B's parity test pins
the two conversion backends together on that gate, and the storage round-trip
test guards the silent `Type` sites. No new harness needed.

## See also

- [README.md](README.md) — the @PLN8 plan + the original open questions.
- `doc/claude/LOFT.md § String formatting` — the format system arc C extends.
- `doc/claude/WASM.md` + `src/wasm.rs` — the `js_sys` bridge arc B's wasm uses.
