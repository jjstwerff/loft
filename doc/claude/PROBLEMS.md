
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Known Problems in Loft

Known bugs, unimplemented features, and limitations in the loft
language and interpreter.  Each entry records the symptom, workaround, and
recommended fix path.

Completed fixes are removed — history lives in git and `CHANGELOG.md`.

## Contents
- [Open Issues — Quick Reference](#open-issues--quick-reference)
- [Unimplemented Features](#unimplemented-features)
- [Interpreter Robustness](#interpreter-robustness)
- [Web Services Design Constraints](#web-services-design-constraints)
- [Graphics / WebGL](#graphics--webgl)

---

## Open Issues — Quick Reference

| # | Issue | Severity | Workaround |
|---|-------|----------|------------|
| 22 | Spatial index (`spacial<T>`) operations not implemented | Low | Compile-time error; use `sorted<T>` or `index<T>` |
| 54 | `json_items` returns opaque `vector<text>` | Low | Accepted limitation; `JsonValue` enum deferred |
| 55 | Thread-local `http_status()` not parallel-safe | Medium | Design constraint — use `HttpResponse` struct |
| 86 | Lambda capture: misleading self-reference error | Low | Mitigated — clear error message at parse time |
| 90 | `fn_call` HashMap lookup per call | Low | Negligible overhead; encode line in `OpCall` if measured |
| 91 | `init(expr)` parameter form missing | Low | Pass default explicitly at call site |
| 135 | Sprite atlas row indexing swap | Low | Cosmetic — affects 2×2 atlas layout |

---

## Unimplemented Features

### 22. Spatial index operations are not implemented

`spacial<T>` in any field or variable type emits a compile-time error:

```
spacial<T> is not yet implemented; use sorted<T> or index<T> for ordered lookups
```

Both first-pass and second-pass parsers reject it, so no program reaches
the runtime "Not implemented" panics.  Test:
`spacial_not_implemented` in `tests/parse_errors.rs`.

**Remaining work:** implement insert, lookup, copy, remove, iteration in
`database.rs` and `fill.rs`.  Iteration first, then remove, then copy.
The spacial index structure (radix tree or R-tree) is already allocated
in the schema; iteration traversal is the main missing piece.

---

## Interpreter Robustness

### 86. Lambda capture produces a misleading codegen self-reference error

**Symptom:** A lambda referencing an outer-scope variable crashed in codegen with

```
[generate_set] first-assignment of 'count' (var_nr=1) in 'n___lambda_0'
contains a Var(1) self-reference — storage not yet allocated
```

**Reproducer:**
```loft
fn test() {
    count = 0;
    f = fn(x: integer) { count += x; };
    f(1);
}
```

The parser created a new local `count` inside the lambda; `count += x`
desugars to `count = count + x` — the RHS reads the same uninitialised
variable, tripping the self-reference guard in `generate_set`.

**Status:** *(mitigated)* — The parser detects the outer-scope reference
and emits `lambda captures variable 'count' — closure capture is not yet
supported` before codegen runs.  Underlying feature (real closure capture)
is tracked as A5.2–A5.5.

---

### 90. `fn_call` HashMap lookup for source line on every call

**Symptom:** Every loft function call performs
`self.line_numbers.get(&self.code_pos)` inside `fn_call`.  Before
stack-trace support, the line lookup only ran during the rare
`stack_trace()` snapshot.

**Root cause:** The source line is not encoded in `OpCall` operands; it
lives in `line_numbers: HashMap<u32, u32>` keyed by bytecode position
and must be looked up at runtime.

**Workaround:** None needed — O(1) amortised lookup, small against the
`Vec::push` + dispatch already in `fn_call`.

**Fix path (if measured significant):** encode the source line as an
extra `u32` operand of `OpCall` in codegen; the `call` handler reads it
and passes it to `fn_call`, eliminating the runtime lookup at the cost
of 4 bytes per `OpCall`.

---

### 91. `init(expr)` parameter form missing

**Symptom:** `init(expr)` on function parameters (dynamic defaults
computed from earlier parameters) is not implemented.

**Scope:** The core struct-field `init(expr)` works correctly —
evaluated once at creation, `$` references resolved, writable after
construction.  Circular detection on struct fields is also in place
(`tests/scripts/72-parse-error-caveats.loft`).  Only the parameter-form
extension is missing.

**Workaround:** Compute the default at the call site and pass it
explicitly.

**Fix path:** in `parse_arguments`, accept `init(expr)` alongside
`= expr`; store the expression in `Attribute.value`; at the call site,
emit the expression when no argument is supplied.

---

## Web Services Design Constraints

### 54. `json_items` returns opaque `vector<text>` — no compile-time element type

**Severity:** Low — accepted design limitation

`json_items(body)` parses a JSON array and returns element bodies as
`vector<text>`.  The compiler cannot verify that the caller's parse
function receives a valid JSON object body rather than an arbitrary
string; a runtime parse error produces a partial zero-value struct
rather than a diagnostic.

**Workaround:** validate the HTTP response status before parsing
(`if resp.ok()`).

**Fix path (deferred):** a `JsonValue` enum (Object / Array / String /
Number / Boolean / Null) gives compile-time structure but at high design
cost.  Target: 1.1+.

See also: [WEB_SERVICES.md](WEB_SERVICES.md).

---

### 55. Thread-local `http_status()` is not parallel-safe

**Severity:** Medium — design trap; do not introduce this API.

An `http_status()` returning the most-recent HTTP status as a
thread-local integer (C `errno` pattern) is incorrect under loft's
parallel execution model.  A `parallel_for` worker calling `http_get`
would corrupt the thread-local of the calling thread.

**Resolution:** return an `HttpResponse` struct from all HTTP functions.
Status is a field on the returned value, not global state.  See
WEB_SERVICES.md Approach B.  This is a design constraint to observe,
not a bug to fix.

---

## Graphics / WebGL

### 135. Sprite atlas row indexing swap

**Severity:** Low — cosmetic.

**Symptom:** in a 2×2 sprite atlas, sprites 1 and 3 appear at
swapped canvas positions when drawn via `draw_sprite`.  The smoke
test (`tests/scripts/snap_smoke.sh`) pixel-samples the affected
corners and confirms the mis-placement is reproducible.

**Root cause:** interaction between `gl_upload_canvas`'s Y-flip
(row reversal during upload, `lib.rs:837`), `draw_sprite`'s
V-coordinate computation (`graphics.loft:773-776`), and the
orthographic projection in `create_painter_2d` (`-2/H`, which also
flips Y).  Two of the three flips cancel; the third lands in an
unexpected quadrant, so row indexing into the atlas is off by one
row.

**Workaround:** arrange sprites in a single row (N×1 atlas) until
the flip sequence is normalised.

**Fix path:** decide a single canonical Y direction (screen-origin
top-left) and remove the compensating flip from one of the three
sites — most naturally the upload, since it's the one introduced
last.  Test: extend `snap_smoke.sh` to assert all four corners of
a 2×2 atlas are placed correctly.

---

## See also
- [PLANNING.md](PLANNING.md) — Priority-ordered enhancement backlog
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — Language design inconsistencies and asymmetries
- [TESTING.md](TESTING.md) — Test framework, reproducing and debugging issues
- [CAVEATS.md](CAVEATS.md) — Verifiable edge cases with reproducers
- [../DEVELOPERS.md](../DEVELOPERS.md) — Debugging strategy and quality requirements
