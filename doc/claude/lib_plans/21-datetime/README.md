<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 21 — `time` library + `DateTime` value type

## Status

**Active 2026-05-25.**  Building the **basics first** to unblock the
trainer app: a pure-loft `lib/time` library over `integer`
epoch-milliseconds (operations arc D + `format_*` text renderers),
needing **zero core changes** and working identically on
interpret / `--native` / wasm.  The **distinct built-in `DateTime`
type** (arc A) + the **built-in `{dt:…}` format specifiers** (arc C) +
`js_sys::Date` wasm delegation are kept as this plan's **"full datetime
support" tail** — fully designed below, scheduled after the basics ship
and the trainer app has continued on them.

Driver: the `training` app (`../personal/training`,
see its `MIGRATION.md § Loft capability gaps`) is date-indexed
(daily wellness, CTL/ATL EWMA over day sequences, weekly rollups,
HRV-by-day) and its B8–B10 routines are blocked on dates alone.
`BROADENING.md`'s Data/ETL gap list names the same hole.

Four decisions are already locked (recorded in `DESIGN_DECISIONS.md`
when arc A lands — see Open questions for the ones still open):

1. **A distinct built-in `DateTime` value type**, backed by `i64`
   epoch-milliseconds — the old-Java-`Date`-over-`long` model, not a
   plain `integer`, so the compiler gives chronological type safety
   and bare `{dt}` auto-renders.
2. **UTC formatting + a fixed-offset helper** for local-day bucketing.
   No IANA timezone database, no DST handling (documented limitation).
3. **Formatting is built into core; operations live in a library.**
   Loft's format specifiers compile straight to opcodes
   (`format_long` in `src/ops.rs`, dispatched in `src/fill.rs`) — there
   is **no per-type hook a library can register**, so `{dt:date}`-style
   rendering *must* be in core.  Everything derivable (parse, add, diff,
   weekday, week number) is pure loft in `lib/time`.
4. **JS-`Date`-aligned semantics so the WASM build borrows the browser.**
   Loft's epoch-ms unit already equals JS `Date`'s, and both map
   epoch-ms → calendar fields through the *proleptic Gregorian
   calendar* — identical results.  So the conversion core has two
   backends with one contract: native pure-Rust civil-calendar math;
   wasm delegates to `js_sys::Date::get_utc_*`.  No calendar math is
   compiled into the WASM binary.

## Goal

Ship a pure-loft `lib/time` library (operations + `format_*` renderers)
covering every date operation the `training` app needs (table below)
with no workarounds — then, as the plan tail, promote it to a distinct
built-in `DateTime` type with built-in `{dt:…}` formatting.

## Effort + design

- **Effort:** H — the new primitive type ripples through ~25 files
  (`Type::Character` appears in 25 files / 104 sites today; a new
  scalar variant is comparable).  Library + formatting are S–M.
- **Design:** ~ partial — principles locked, token vocabulary and
  type-operator rules still open (see Open questions).
- **Last touched:** 2026-05-25

## Design principles

- **`DateTime` = `i64` epoch-ms, JS-`Date`-aligned.**  Same unit/epoch
  as `now()` and JS `Date`.  Null sentinel `i64::MIN`, matching the
  long-integer null pattern.  Cross-backend parity is *structural*, not
  best-effort: the same epoch-ms renders identically on
  interpret / `--native` / wasm because all three implement the one
  Gregorian mapping.
- **Thin WASM.**  One conversion contract `epoch_ms ↔ {y,mo,d,h,mi,s,wday}`,
  two impls: native `days_from_civil`/`civil_from_days`; wasm
  `js_sys::Date` UTC getters.  A cross-mode parity test pins them
  together.
- **Small, bounded surface.**  This is a "basic" type (user's word):
  no `Duration` type, no calendar arithmetic beyond day/week/second
  steps, no locales.  `DateTime − DateTime` yields plain milliseconds;
  the library wraps that as `days_between` etc.

## Sub-arcs

| Item | Source | Status |
|---|---|---|
| **D** — `lib/time` operations + `format_*` renderers (pure loft, over `integer` epoch-ms) | `lib/time/` (new package) | **BASICS — SHIPPED 2026-05-25.**  Validated on interpreter + `--native` (run, all asserts pass) and wasm32-wasip2 (compiles clean).  Test: `tests/docs/32-time.loft` (CI three-backend) + `lib/time/tests/01-basics.loft` (package smoke). |
| **E** — port the training app's date-dependent routines onto `lib/time` | `../personal/training/loft/` | BASICS — next |
| **A** — distinct `DateTime` variant in the `Type` enum + parser + type rules | `src/data.rs`, `src/typedef.rs`, `src/parser/` | FULL — deferred (plan tail) |
| **B** — native + wasm conversion backends (`js_sys::Date`) + cross-mode parity test | `src/ops.rs`, `src/wasm.rs` | FULL — deferred (basics use pure-loft math; B replaces it with the two-backend contract) |
| **C** — built-in `{dt:…}` format specifiers | `src/ops.rs`, `src/fill.rs`, `src/state/codegen.rs` | FULL — deferred (basics use `time::format_*`) |

## Phase ordering

**Basics (now — unblocks the trainer app, zero core changes):**

1. **D** — write `lib/time` as pure loft over `integer` epoch-ms:
   civil-calendar conversion (Hinnant `days_from_civil` /
   `civil_from_days`) + the operations and `format_*(…) -> text`
   renderers in the coverage table.  Tests against hand-computed
   goldens; runs on interpret / `--native` / wasm unchanged.
2. **E** — port training B8–B10 (HRV attribution, `hrv_predict`,
   goals/phase/schedule) onto `lib/time`; verify against the Python
   parity oracle.

**Full datetime support (plan tail — after the basics are in use):**

3. **B** — replace the library's pure-loft conversion with a two-backend
   contract: native pure-Rust civil-calendar; wasm `js_sys::Date` UTC
   getters.  Cross-mode parity test pins them.
4. **C** — wire the `{dt:…}` format-opcode tokens using B (de-risk on a
   plain `integer` first: `{ms:date}`).
5. **A** — add the distinct `Type::DateTime`; bare `{dt}` auto-picks the
   renderer; define the operator rules.  Largest blast radius, done
   last.  `time::*` signatures migrate `integer` → `DateTime` here — a
   type-only change to the library, its bodies unchanged.

## Training-app coverage — every API traces to a real use

Derived from grepping `../personal/training/python/*.py` (the parity
oracle).  The plan is "enough for the training app" iff every row has a
home in A–D.

| Training-app need (Python) | Covered by |
|---|---|
| `date.fromisoformat("YYYY-MM-DD")` (33×) | `time::parse(text) -> DateTime` |
| `datetime.fromisoformat(...)` w/ time (4×) | `time::parse` (accepts `YYYY-MM-DD[ T]HH:MM[:SS]`) |
| `datetime.fromtimestamp(epoch)` (5×) | `now()` returns `DateTime`; epoch-ms ctor `time::from_millis(i)` |
| `.isoformat()` (99×) | `time::format_date` / `time::format_iso` (basics); `{dt:date}`/`{dt:iso}` (full) |
| `.strftime("%Y-%m-%d")` / `"%H:%M"` / `"%a"` (4×) | `time::format_date` / `format_time` / `weekday_name` (basics); `{dt:…}` (full) |
| `date.today()` / `datetime.now()` (38×) | `now()` (instant) + `time::today(offset_min)` (local day) |
| `+ timedelta(days=N)` / `weeks=N` (many) | `time::add_days(dt,n)` / `time::add_weeks(dt,n)` |
| `(a - b).days` | `time::days_between(a,b) -> integer` |
| `(a - b).total_seconds()` (7×) | `(a - b)` → ms; `time::seconds_between(a,b)` |
| `d.weekday()` (6×) — week starts Monday | `time::weekday(dt) -> integer` (0=Mon … 6=Sun) |
| `d - timedelta(days=d.weekday())` (week start) | `time::start_of_week(dt)` |
| `isocalendar()` → (year, week) | `time::iso_year(dt)` / `time::iso_week(dt)` |
| local "today" via `zoneinfo` | `time::local_day(dt, offset_min)` / `time::today(offset_min)` — fixed offset, **no DST** |
| ordering / comparison of dates | built-in `==` `<` `>` on `DateTime` (chronological) |

## Built-in format tokens (arc C) — proposal

Mirror what JS `Date.toISOString()` / field getters produce; keep the
set small.  Bikeshed in Q6.

| Spec | Renders | JS analogue |
|---|---|---|
| `{dt}` | `2026-05-25 14:30` | — (loft default) |
| `{dt:date}` | `2026-05-25` | `toISOString().slice(0,10)` |
| `{dt:time}` | `14:30` | `getUTCHours`:`getUTCMinutes` |
| `{dt:datetime}` | `2026-05-25 14:30:07` | combined |
| `{dt:iso}` | `2026-05-25T14:30:07Z` | `toISOString()` |
| `{dt:wday}` | `Mon` | `getUTCDay()` → name |

## `lib/time` operations (arc D) — idiomatic loft names

Names chosen for loft, **not** transliterated from Python.

- **Construct / parse:** `parse(text) -> DateTime`,
  `from_millis(integer) -> DateTime`, `from_ymd(y,mo,d) -> DateTime`,
  `today(offset_min) -> DateTime`.
- **Step:** `add_days(dt,n)`, `add_weeks(dt,n)`, `add_seconds(dt,n)`.
- **Difference:** `days_between(a,b) -> integer`,
  `seconds_between(a,b) -> integer`.
- **Fields:** `year`, `month`, `day`, `hour`, `minute`, `second`,
  `weekday(dt) -> integer`, `iso_year`, `iso_week`.
- **Boundaries:** `start_of_day(dt)`, `start_of_week(dt)` (Monday).
- **Local (fixed offset, no DST):** `local_day(dt, offset_min)`.

All pure loft over the `DateTime` i64 + the arc-B conversion (exposed to
loft as the field accessors).  No native code in the library.

## Cross-backend testing — all four backends gated

The basics ship with two test files:
`lib/time/tests/01-basics.loft` (package smoke, travels on extraction)
and `tests/docs/32-time.loft` (monorepo docs).  Verified 2026-05-25,
`time` is **executed + asserted green on every backend**:

| Backend | Gate | Status for `time` |
|---|---|---|
| Interpreter | `wrap.rs::library_suite` (`loft test`) + docs `wrap.rs::dir` | runs + asserts pass |
| `--native` | `native.rs::native_library_suite` (`loft --native test`) + docs `native.rs` | runs + asserts pass |
| browser `feature="wasm"` | `html_wasm.rs::wasm_library_suite` → `--html` + `tools/wasm_repro.mjs` (Node) | **runs + asserts pass** |
| wasm32-wasip2 | `html_wasm.rs::wasm_library_suite` → rustc + wasmtime | **runs + asserts pass** |

**The WASM library gate now exists** — `tests/html_wasm.rs::wasm_library_suite`
(added 2026-05-25), closing the gap that
[`../12-library-extraction/`](../12-library-extraction/) § WASM gate
flagged.  It iterates `lib/*/tests/*.loft`, and for each test that
declares a `fn main()` (the wasm entry point) runs it under **Node**
(browser `feature="wasm"` via `--html`) and **wasmtime** (`wasm32-wasip2`)
when those tools are available — self-skipping otherwise, so it's a no-op
on a box with neither.  `loft --native-wasm test` is NOT a building block:
the `test` subcommand runs the interpreter and ignores `--native-wasm`.

Skip-lists (`LIB_PKGS_WASM_SKIP` / `LIB_TESTS_WASM_SKIP`) park backends
that genuinely can't run: `server` (no browser/WASI sockets — a real
platform limit), `imaging` ([@P321c](../../PROBLEMS.md) — store-mutating
`#native` codegen; the right browser fix is a JS bridge over the
browser's own image codec, not a bundled Rust PNG stack), and `world`
([@P334](../../PROBLEMS.md) — traps on both wasm runtimes though it
passes interp + native; surfaced by this gate).

**Arc-B note:** when the `js_sys::Date` browser delegation lands, its
parity test rides this same gate — assert the Node (browser) run's output
matches the native golden, since that's the path the JS fallback lives on.

## Open design questions

> **Resolved 2026-06-14 in [DESIGN.md](DESIGN.md)** — the code-grounded
> implementation design for the full-datetime tail (arcs B/C/A): Q1 distinct
> `Type::DateTime` (silent integer arithmetic proven impossible by construction
> via `can_convert`), Q2 constructor-only, Q3 comparison opt-in / arithmetic
> forbidden, Q6 token vocabulary + bare-`{dt}` default.  Originals kept below for
> context.

1. **Distinct `Type::DateTime` vs a tagged `Integer`?**  Locked to
   *distinct* by the user, but confirm the storage/codegen route:
   reuse the long-integer storage path (`i64`, `i64::MIN` null) and add
   only the static-type identity + format-default + operator rules,
   rather than a wholly new storage class.
2. **Literals in source, or constructor-only?**  Likely
   constructor-only (`time::parse("…")` / `now()`); no `DateTime`
   literal syntax in the lexer.  Confirm.
3. **`DateTime − DateTime` → ?**  Proposal: plain `integer`
   milliseconds (no `Duration` type — keeps it "basic"); library wraps
   as `days_between`/`seconds_between`.  `DateTime + integer` — allow
   (ms offset) or force `time::add_*`?  Leaning: forbid bare `+ integer`
   (ambiguous unit), require library steps.
4. **`weekday` as `integer` or a `Weekday` enum in `lib/time`?**
   **RESOLVED in basics:** `integer` 0=Mon…6=Sun (plus `weekday_name`
   for the `%a` need).  An enum can be added later without breaking
   callers.
5. **Library name: `lib/time` vs `lib/date`?**  **RESOLVED: `time`**
   (covers both date and time-of-day); lives in `loft-libs-core`.
6. **`{dt:…}` token vocabulary** (table above) + the bare-`{dt}`
   default (`datetime` to the minute? full `iso`?).  Still open — the
   basics ship `time::format_*` text renderers; the `{dt:…}` specifiers
   are the arc-C full-support tail.
7. **Parse strictness** — **RESOLVED in basics:** accept `YYYY-MM-DD`,
   optional ` `/`T` + `HH:MM[:SS]`, optional `Z`; anything malformed
   returns null.

## Cross-arc dependencies

- **lib_plans/12-library-extraction** — `lib/time` is on the extraction
  roster in the **`loft-libs-core`** chunk (Tier A — pure-loft, no
  `native/` dir, zero compiler-crate coupling).  The training app
  consumes it as a registry dependency (`loft install time`), so it
  must be a published library, not monorepo-internal.  **Boundary
  note:** the built-in `DateTime` type (arc A) is a language PRIMITIVE
  in the compiler crate, NOT library code — it does not count against
  plan-12's drain goal, exactly as `integer`/`text` don't.  Keep all
  *operations* in `lib/time` (extractable); only the type identity +
  format dispatch live in core.
- **lib_plans/future/03-lazy-stdlib** — if `lib/time` ever grows a
  native bridge, it inherits the lazy-load trigger pattern; the
  pure-loft design avoids needing it.

## See also

- `doc/claude/LOFT.md § String formatting` — the format system arc C extends.
- `doc/claude/WASM.md` + `src/wasm.rs` (`host_call` / `js_sys`) — the
  browser-bridge mechanism arc B's wasm backend uses.
- `doc/claude/BROADENING.md` — Data/ETL gap list naming the date hole.
- `../personal/training/MIGRATION.md` — the driving consumer + parity oracle.
- ROADMAP.md `F` table row `TIME.1`.
