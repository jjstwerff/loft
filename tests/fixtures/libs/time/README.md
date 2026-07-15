<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# time — date/time operations for loft

```sh
loft install time
```

Pure-loft date / time arithmetic on millisecond-since-epoch
integers — the same model as JavaScript's `Date`.  Identical results
on the interpreter, `--native`, and WASM.

Two layers, use whichever fits: a **`value struct DateTime` / `Duration`** for
first-grade, type-safe, zero-cost dates (0.2.0), on top of the original
`integer`-epoch functions (unchanged — the 0.1.0 surface still works).

## DateTime / Duration (0.2.0)

`DateTime` and `Duration` are zero-cost `value struct`s over the same epoch-ms
`integer` — so they add **no** heap allocation as a record field or vector
element (a `vector<DateTime>` costs the same as `vector<integer>`), yet they are
a distinct type: `dt + 5` is a compile error, only `dt + hours(2)` steps a time.

```loft
use time;

d1 = time::datetime(2026, 7, 8, 12, 0, 0);   // or time::date(y, mo, d)
d2 = "2026-07-08T13:30:00" as DateTime;        // total best-effort parse
span = d2 - d1;                                 // -> Duration
later = d1 + time::hours(2);                     // typed stepping
if d1 < d2 { }                                   // all six comparisons
n = d1.year(); mm = d1.month(); w = d1.weekday_name();
s = "{d1:date}";   // 2026-07-08   ·  "{d1:iso}" · "{d1:time}" · "{d1}" · "{span}"
```

- **Construct**: `date(y,mo,d)`, `datetime(y,mo,d,h,mi,s)`, `ms as DateTime`,
  `"…" as DateTime`, `now() as DateTime`.
- **Read (methods)**: `year month day hour minute second weekday iso_year
  iso_week weekday_name month_name to_millis`.
- **Operators**: `< <= > >= == !=`; `dt - dt -> Duration`; `dt + Duration ->
  DateTime`; `dt.minus(Duration)`.
- **Duration**: `milliseconds seconds minutes hours days weeks`; `+ - *`,
  comparisons, `negate`, `total_millis/seconds/minutes/hours/days`.
- **Format**: `{dt}` `{dt:date}` `{dt:time}` `{dt:seconds}` `{dt:iso}`
  `{dt:wday}` `{dt:month}` · `{dur}` → `[-]H:MM:SS`.
- **Nullability**: a `value struct` has no null, so a fallible parse cannot
  return `DateTime` *and* signal failure.  Keep failure at the integer level —
  `parse(s)` returns null on bad input — then wrap: `ms = parse(s); if !ms
  { … } else { dt = ms as DateTime }`.  `"…" as DateTime` is a total best-effort
  parse for the path where a bad date need not be caught.

## The integer-epoch API (0.1.0, still current)

- **Construct / parse**: `from_ymd`, `from_millis`, `parse`, `today`.
- **Field extraction (UTC)**: `year`, `month`, `day`, `hour`, `minute`,
  `second`, `weekday` (0=Mon..6=Sun), `iso_year`, `iso_week`.
- **Step**: `add_days`, `add_weeks`, `add_seconds`.
- **Difference**: `days_between`, `seconds_between`.
- **Boundaries**: `start_of_day`, `start_of_week` (Monday).
- **Local time**: fixed-offset (minutes) — `to_local`, `local_day`,
  `today(offset_minutes)`.  No DST, no tz database.

## Why this package exists

Every game needs frame timing, run-length tracking, and date display
(scores, save-file timestamps, daily-challenge boundaries).  Bundling
those primitives in a small, dependency-free, identically-behaving
package means a game can drop it in without pulling in a graphics or
windowing stack.

The proleptic Gregorian calendar implementation is Howard Hinnant's
days_from_civil / civil_from_days — field-for-field match with
`js_sys::Date`, so WASM consumers see the same values as native.

## Tests

```sh
cd time && loft --interpret --tests tests
```

`tests/01-basics.loft` covers the integer-epoch API (construction, field
extraction across the year, leap-year boundaries, week math, ISO week
numbering, local-day bucketing under several offsets).
`tests/02-datetime.loft` covers the `DateTime` / `Duration` value types
(construction, fields, operators, typed arithmetic, formatting, conversions,
and zero-cost use as vector elements).  All goldens are hand-computed and
verified identical on `--interpret` and `--native`.
