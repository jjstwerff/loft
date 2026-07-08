<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN47 — Binary file I/O type matrix (write ↔ read round-trip)

**Status: future — but the frozen matrix is stale; empirical inspection
2026-07-09 (both backends) revised it.** W7 (text) quietly landed since the
freeze; **W8 silently loses data and W9 crashes** — see § Empirical state below.
Absorbs the open canonical-object-serialization gap formerly tracked as
[@P289](../../PROBLEMS.md); that P-issue is now this plan's design half.

## Empirical state — inspection 2026-07-09 (interpret + native)

A round-trip probe ([`probes/roundtrip-2026-07-09.loft`](probes/roundtrip-2026-07-09.loft),
run per-block since W9 crashes) — hand-verified values on BOTH backends:

| Row | Probe | interp | native | Verdict |
|---|---|---|---|---|
| W0–W6 scalars | `tests/scripts/115-snapshot-roundtrip`, `20-binary` | ✅ | ✅ | shipped (as the plan predicted) |
| **W7 text** | `f += "hello"` → `f#read(5) as text` | ✅ `'hello'` | ✅ `'hello'` | **LANDED since freeze** — the explicit-count `f#read(N) as text` round-trips; matrix's "⚠️ no length prefix" is out of date |
| **W8 vector** | `f += [10,20,30]` → `f#read(3) as vector<integer>` | ⚠️ `len=0` | ⚠️ `len=0` | **SILENT DATA LOSS** — reads back empty, no error. `f#read(N)` count-vs-bytes semantics + the `f += vector` write need deciding/fixing |
| **W9 struct** | `f += Point{x,y,tag}` → `f#read as Point` | ❌ CRASH | ❌ CRASH | **PANIC** `index out of bounds: len 3, index 65535` at `src/database/io.rs:273` (a struct field resolves to the `u16::MAX` unresolved-type sentinel inside the field-width `sum()`). NOTE: `f#read as MyStruct` for a plain scalar struct **is already wired in the parser** (`src/parser/objects.rs` rejects only collection-field structs — "use a plain struct for serialisation"); the fault is in the runtime write/read-data path, so W9 is a **bug to fix**, not a feature to build |

**Bottom line:** not "almost done" in the benign sense. W7 shipping is real
progress, but W8 (silent corruption) and W9 (hard crash) are exactly the S-tier
failure class this plan exists to catch. Revised remaining work, in priority:

1. **W9 — root-cause the crash first.** Start at the *producer* of the bad
   `u16::MAX` type_nr, not `io.rs:273` which only consumes it. The wiring is
   present (parser + runtime dispatch), so this is a delivery/resolution bug.
2. **W8 — fix the empty-read.** `f += vector<integer>` + `f#read(N) as
   vector<T>` yields `len=0`; decide whether `(N)` counts elements or bytes and
   make the write emit recoverable data (not a bare storage handle).
3. **W7 — DONE; just lock it in.** Add the cross-mode test; only the matrix
   harness is missing here.
4. Phase 00's `tests/binary_io_matrix.rs` harness is still unbuilt.

The old "The matrix" table below is the *frozen design*; the row above is the
*measured reality*. When resuming, reconcile the two (flip W7 to ✅, retag W8/W9
as bug rows).

## Goal

Validate that **every loft value type** survives a `f += value`
(write) → reopen → `f#read as T` (read) round-trip, for **every binary
format** (LittleEndian / BigEndian, plus TextFile where it applies),
with **interp/native byte-identical** results — and **extend** the file
API to round-trip the types that currently can't (length-prefixed
text / vectors, per-field structs, `f#read as MyStruct`).

This is the file-format equivalent of the @PLAN14 tuple matrix and the
16/18/19/20 validation plans: a value-tier **S** initiative (catch
backend divergence + silent data loss).  Binary I/O is the only stdlib
surface where a width/endianness mismatch produces *plausible-looking
garbage* with no error — exactly the failure class S exists for.

## Why now — what the recent work leaves half-finished

A wave of binary-I/O fixes just landed and deserves a matrix to lock
them in and find the siblings they didn't cover:

- **@P293** (closed 2026-05-19) — `f#read as u32` read 8 bytes; fixing
  the `u32` `size(4)` exposed three latent narrow-key hash bugs.  The
  width-by-cast path (`u8`/`u16`/`u32`/`i8`/`i16`/`i32`) is now correct
  but only spot-tested.
- **`+=` append semantics + `file.sync()`** (landed 2026-05-20, see
  [CHANGELOG](../../../../CHANGELOG.md)) — `f += value` now appends;
  explicit `f#next = N` overwrites in place.  The interaction of
  append/seek/sync with each value type is untested as a matrix.
- **@P289** (this plan) — `f += text`/`vector<T>` write raw bytes with
  no length prefix (callers track size out-of-band); `f += plain_struct`
  writes the storage handle, NOT field values; `f#read as MyStruct` is
  unimplemented.

The dogfood consumers that drove these (`lib/world` snapshot save/load,
`lib/graphics` GLB export, `single_port_server` world persistence) all
hand-roll field-by-field serialization today — the matrix tells us
when they can stop.

## The matrix

Three axes.

### Axis 1 — value type written / read

| ID | Type | Bytes | Today | Notes |
|---|---|---|---|---|
| W0 | `integer` (i64) | 8 | ✅ write+read | Default integer width |
| W1 | `i32` / `u32` | 4 | ✅ (since @P293) | u32 ≥ 2³¹ round-trips via raw bytes, reads back negative i64 in expressions — document the signed-range caveat |
| W2 | `i16` / `u16` / `short` | 2 | ✅ | |
| W3 | `i8` / `u8` / `byte` / `boolean` | 1 | ✅ | |
| W4 | `character` | 4 | ✅ | u32 codepoint |
| W5 | `float` (f64) | 8 | ✅ (since @P284 read-guard) | |
| W6 | `single` (f32) | 4 | ✅ | |
| W7 | `text` | var | ⚠️ raw bytes, no length prefix | **P289**: needs a length-prefix convention so `f#read as text` knows the byte count |
| W8 | `vector<scalar>` | var | ⚠️ raw concat, no count | **P289**: length-prefix or explicit `f#read(N) as vector<T>` (the simpler half — N known at read site, GLB-style) |
| W9 | `struct` of scalars | sum | ❌ `f += s` writes the storage handle, not fields; `f#read as S` unimplemented | **P289** core: per-field walk both directions |
| W10 | `struct` w/ text / vector field | var | ❌ | **P289** stretch: collection-bearing struct; reject in the legacy unprefixed path with a clear diagnostic if not supported |
| W11 | `vector<struct>` / nested struct | var | ❌ | **P289** stretch |

### Axis 2 — format

| ID | Format | Notes |
|---|---|---|
| F1 | `LittleEndian` | Primary; little-endian byte order |
| F2 | `BigEndian` | Byte order flips; scalars must mirror F1 |
| F3 | `TextFile` | Text-mode write/read; W7 only (binary scalars are LE/BE only) |

### Axis 3 — access pattern (cross-cuts the append/seek/sync work)

| ID | Pattern | Notes |
|---|---|---|
| A1 | sequential append (`f += a; f += b`) | Default; the new append semantics |
| A2 | explicit-offset overwrite (`f#next = 0; f += …`) | In-place; fixed-slot header idiom |
| A3 | truncate-then-write (`f.set_file_size(0); f += …`) | Snapshot-replace idiom |
| A4 | write → `f.sync()` → reopen → read | Durability round-trip |

Every populated cell asserts **interp == native** byte-identical via
the `cross_mode!` harness shipped by closed @PLAN14.

## Phase layout

| Phase | Scope | Outcome |
|---|---|---|
| [00 — matrix freeze + harness wiring](00-matrix.md) | (tables) | Frozen matrix; `tests/binary_io_matrix.rs` binary; smoke cell.  No production change. |
| 01 — scalar baseline (W0–W6) | W0–W6 × F1/F2 × A1–A4 | Should be mostly ✅; locks in @P293 width-by-cast + @P284 float-read + the append/seek/sync semantics across all scalar widths + both endians. |
| 02 — text round-trip (W7) | W7 × F1/F3 | **P289** half A: choose + implement the length-prefix convention for `text` so `f += t` / `f#read as text` round-trips without out-of-band length tracking.  TextFile mode (F3) reads to EOF / by explicit count. |
| 03 — vector round-trip (W8) | W8 × F1/F2 | **P289** half B: `f#read(N) as vector<T>` (count known at read site — the simpler, immediately-useful GLB-style form) first; length-prefixed `vector<T>` second. |
| 04 — struct serialization (W9) | W9 × F1/F2 | **P289** core: `f += my_struct` walks fields in declared order; `s = f#read as MyStruct` reverses it.  Extend `dispatch_read_data` to walk the struct layout. |
| 05 — collection-bearing + nested (W10/W11) | W10, W11 | **P289** stretch: text/vector struct fields + nested structs; OR a clean "unsupported in unprefixed path" diagnostic if deferred. |
| 06 — freeze + doc | — | Update [STDLIB.md § File I/O](../../STDLIB.md) + the canonical-serialization section; retire the @P289 PROBLEMS.md row. |

## Pre-flight gate

Phase 00 runs a quick survey (one cell per W-row × F1) to measure the
real pass rate before committing to phases 02–05.  If the scalar rows
(W0–W6) are all green and only W7–W11 fail (the known P289 gaps), the
matrix is doing its job: phases 01 becomes lock-in tests, 02–05 are the
feature build-out, and W10/W11 may close as deferred if the GLB/world
consumers don't need them.

## Acceptance for the whole plan

- Matrix in [00-matrix.md](00-matrix.md) fully populated (✅ / ❌ / N/A).
- Every scalar cell (W0–W6 × F1/F2 × A1–A4) has a `cross_mode!` test in
  `tests/binary_io_matrix.rs`.
- `f += text` ↔ `f#read as text` round-trips (W7) without out-of-band
  length tracking.
- `f#read(N) as vector<T>` round-trips (W8 half B).
- `f += my_struct` ↔ `f#read as MyStruct` round-trips for scalar-field
  structs (W9).
- @P289 row removed from PROBLEMS.md; STDLIB.md serialization section
  updated to describe the shipped surface.

## Out of scope

- **JSON serialization** — separate text-based path (`json_parse` /
  JsonValue, QUALITY.md Q-tier); this matrix is the binary file API.
- **mmap-backed stores** — the `mmap` feature has its own write path;
  the matrix targets the default `std::fs` path.
- **WASM VirtFS divergence** — covered by the WASM file-I/O tests
  ([WASM.md](../../WASM.md) FS-A..FS-F); this matrix is interp+native
  on the host.

## Cross-references

- [@P289 (PROBLEMS.md)](../../PROBLEMS.md) — the gap this plan owns.
- [STDLIB.md § File I/O](../../STDLIB.md) — current API + canonical-
  serialization design notes.
- `src/state/io.rs` — `write_file` / `read_file` / `dispatch_read_data`
  (interp); `src/codegen_runtime.rs` — `OpWriteFile` / `OpReadFile`
  (native); `src/parser/objects.rs` — `f#read as T` width/size inference.
- [@PLAN14](../finished/14-tuple-validation/README.md) — the
  `cross_mode!` harness this plan reuses.
- Recent landed work: @P293 (narrow-width read), @P284 (float-read
  guard), `+=` append semantics + `file.sync()` (CHANGELOG 0.8.6).
