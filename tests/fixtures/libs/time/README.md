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

## What's in it

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

`tests/01-basics.loft` covers construction, field extraction across
the year, leap-year boundaries, week math, ISO week numbering, and
local-day bucketing under several offsets.
