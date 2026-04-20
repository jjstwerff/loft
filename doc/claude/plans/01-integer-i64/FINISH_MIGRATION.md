# Finish the integer → i64 migration

## Context

The `int_migrate` branch lands the full semantic migration.  After
commit `74aefb4` the workspace suite is **green** (0 failures across
all categories, including native codegen, WASM, and HTML export).
The core migration is shipping: `integer` is i64 at rest and at
runtime, arithmetic is i64 end-to-end, the four narrow-storage
widths (`u8`/`u16`/`i8`/`i16`) + the 4-byte alias (`i32`) are
honoured by `Parts::{Byte, Short, Int}` and round-trip on both
interpreter and native paths.

What remains is **cleanup** — the "Phase 2c round 10b/10c + post-
migration hardening" work called out in the README as not-started.
This file captures everything needed to close the initiative.

## Status of the phase-level plan

| Phase | Status now |
|---|---|
| 00 — null enforcement audit | **Done** (pre-merge) |
| 01 — checked arithmetic | **Done** (`925ee36`) |
| 02 — i64 storage | **Done** (this branch) |
| 03 — u32 type | **Done** (via Phase 2a) |
| 04 — deprecate `long` | **Partial** — keyword aliases `integer`, `l` suffix deprecated.  Remove needed. |
| 05 — opcode reclamation | **Not started** |
| 06 — spec docs | **Not started** |

Remaining failure count: **0** (down from 59 at the start of this
session).  Eleven single-point fixes in this session plus the
earlier 2c rounds 1-10 turned the migration from "in-flight with
silent data corruption risk" into "semantically complete but with
duplicate code-paths to prune."

## What remains — prioritised

### A — Post-migration hardening (ship first, small cost, prevents
regressions)

These are low-effort gates that cement the migration so future work
can't silently reintroduce the bugs this branch chased.

1. **Codegen size-consistency assertion** (CATEGORY_C_PLAN's
   Stage 3).  In `src/state/codegen.rs` wherever `OpSetInt*` /
   `OpGetInt*` / `OpSetByte/Short` / `OpSetInt4` are emitted,
   pair the emission with `debug_assert_eq!(emitted_byte_width,
   expected_field_width)` so any future post-round regression fails
   loudly at compile time instead of producing a silently corrupt
   file / misaligned record.

2. **Binary-format writer audit + lint** (`lib/graphics/src/glb.loft`
   was patched in `74aefb4`, but siblings exist).  Sweep:
   - `lib/graphics/src/png.loft`
   - `lib/graphics/src/glb.loft` (done — verify no remaining `f +=
     <integer>` without `as <width>`)
   - Any test or library writing a binary protocol via `f +=` on a
     scalar integer expression.

   Add a `loft` static-check (or manual checklist) that flags
   `f += N` / `f += var` where the RHS is an unqualified `integer`
   going into a file opened with `BigEndian` / `LittleEndian`
   format.  The user should be forced to write
   `f += N as <width>`.  Alternative: extend the parser to
   DEPRECATION-WARN on `f += integer_expr` where the file is in a
   non-TextFile format.

3. **Content-type mapping doc**.  Add a three-line table in
   `doc/claude/INTERMEDIATE.md` (or similar) documenting the
   `field.content` → `Key.type_nr` → `Content::X` mapping that
   cost this session hours.  The shift of 0-based `field.content`
   (integer=0, character=6) vs 1-based `Key.type_nr`
   (integer=1, character=7) is non-obvious and bit us twice.

### B — Round 10b follow-up: deduplicate `Op*Long` family

**Status (2026-04-20):** partial — `OpAbsLong` is removed (proof-of-
concept deletion via `regen_fill_rs`).  A 22-op bulk batch (arithmetic
+ bitwise + comparison + conversion Long ops) was attempted and
passed `wrap` / `issues` / `parse_errors` / `clippy` / `fmt` but
regressed `moros_glb_cli_end_to_end`: the native-CLI run SIGSEGVs at
runtime (opcode dispatch) during JSON-driven moros_render pipeline
execution.  Minimal reproducers with long arithmetic in isolation
don't crash — the interaction is somewhere deeper (possibly
arg-conversion across lib boundaries that still see `long` types but
dispatch `OpXxxInt`).  The 22-op batch was reverted; B stays at 1/26.

Resume path: (a) identify the pc=84102 bytecode site in a freshly-
compiled moros_glb run with `LOFT_LOG=static` and diff the emitted
function body vs pre-B to find the pattern that changes; (b) when
resolved, re-apply the 22-op batch with the fix, then the remaining
~4 special-case ops (`OpConvLongFromInt`, `OpFormatLong`, etc.).

Per `05-opcode-reclamation.md`, with `Type::Long` collapsed to
`Type::Integer`, ~26 opcodes are duplicates:

- Arithmetic: `OpAddLong`, `OpMinLong`, `OpMulLong`, `OpDivLong`,
  `OpRemLong`, `OpAddLongNn`, `OpMinLongNn`, `OpMulLongNn`,
  `OpDivLongNn`, `OpRemLongNn`, `OpNegLongNn`.
- Bitwise: `OpLandLong`, `OpLorLong`, `OpEorLong`, `OpSLeftLong`,
  `OpSRightLong`.
- Comparison: `OpEqLong`, `OpNeLong`, `OpLtLong`, `OpLeLong`,
  `OpConvBoolFromLong`.
- Conversion: `OpConvLongFromInt`, `OpConvLongFromSingle`,
  `OpConvLongFromFloat`, `OpCastIntFromLong`, `OpCastLongFromInt`.

Path to delete:

1. Route every `default/*.loft` / `lib/*.loft` use of `OpXxxLong`
   through the `OpXxxInt` equivalent (they're byte-identical
   post-2c).
2. Remove the `ops::op_xxx_long` functions from `src/ops.rs`
   (after the parser stops emitting the Long-family ops).
3. Remove the fill.rs dispatch entries — reclaims opcode slots.
4. Update the `OPERATORS` table size constant in `src/fill.rs:10`.

Estimate: 2-3 hours of mechanical sweeps + one suite run.
**Risk**: low — each `OpXxxLong` body already delegates to the
i64 arithmetic.  The deduplication is a rename at call sites.

### C — Round 10c: remove `Type::Long` and widen default range

Currently:

- `Type::Long` is still a distinct enum variant in `src/data.rs`.
- `Type::Integer` has a default range of `i32::MIN+1 .. i32::MAX`
  (see `src/data.rs:32`, `src/parser/definitions.rs:1068`), so a
  literal like `9_000_000_000` is auto-promoted to `Type::Long`.
- `Type::Long ↔ Type::Integer` conversion is implicit — they're
  the same thing at runtime, but the code paths stay split.

Steps to close:

1. Widen the default integer range to full i64 — literal
   `9_000_000_000` should type-check as `Type::Integer`, not
   promote to `Type::Long`.  Touches `src/parser/definitions.rs`
   + `src/data.rs:I32` static.
2. Change `Type::Long` into a deprecated alias (route-through in
   the parser / type-checker) or delete the variant and collapse
   all match arms.
3. Clean up ~60 `Type::Long => …` match arms across the codebase
   (`grep -rn "Type::Long" src/ | wc -l` currently: check before
   starting — used to be ~60).
4. Delete the `long` keyword from the parser's keyword table
   (`src/lexer.rs`) with a final deprecation sweep of `.loft`
   files.

Estimate: 3-5 hours; higher risk than 10b because the `Type` enum
touches the parser, type-checker, bytecode, and native codegen.
**Regression gate**: the existing `wrap::auto_convert` +
`wrap::integers` suites cover the surface; any match-arm miss
shows up as a missing case in the dispatch.

### D — Persisted-database migration tool (`--migrate-i64`)

Called out in README but not landed.  Any on-disk
loft-stored database from pre-2c uses i32 integer storage and
will read back wrong values on the post-2c runtime.  The tool:

- Reads the pre-2c `.loftdb` (or whatever the extension is);
  probes type table for `Parts::Integer` fields.
- Rewrites each record into a post-2c layout (i64 field width).
- Updates the `.loftc` cache-format version constant.

Estimate: 4-6 hours.  Required before shipping a `1.0` if any
user has a persisted database.  **If no user has one yet**,
punt to "document the incompatibility in RELEASE.md and require
fresh-DB start" — cheaper and we can defer the tool.

### E — Phase 6 docs: record the invariants

Write the post-landing documentation into:

- `doc/claude/LOFT.md` — update the integer / long / narrow-type
  sections.  Document the i64-everywhere model and the narrow-
  storage aliases (`i32` = `integer size(4)`).
- `doc/claude/PROBLEMS.md` — close the C54 entry, move to the
  "Closed" section of CHANGELOG.md.
- `doc/claude/CAVEATS.md` — add entries for the post-migration
  downsides: file-format `as <width>` requirement, cdylib
  vector<integer> element storage quirk, stack/frame-size
  increase.
- `CHANGELOG.md` — add a `0.9.x / 1.0.0` milestone entry for the
  integer-i64 migration with a brief summary (3-5 sentences).
- `doc/claude/INCONSISTENCIES.md` — remove any C54-related entries
  that the migration now resolves.

Estimate: 2-3 hours of writing; no code changes.

## Recommended execution order

1. **A1** (codegen assertions) — lowest risk, locks in the gains.
   Same session as this plan.
2. **A2** (binary-format audit) — same session, short sweep.
3. **B** (Op*Long dedup) — next session; mechanical; ~3 hours.
4. **C** (Type::Long removal) — session after B; higher risk.
5. **E** (docs) — after C, when the invariants are final.
6. **D** (migration tool) — defer until a user has a persisted DB
   that needs it, OR lock it behind "must start with fresh DB"
   in RELEASE.md.

## Downsides to document

The migration ships real gains but also real costs.  Record
these in `CAVEATS.md` so users aren't surprised:

- **Memory**: every `integer` slot doubles (4 → 8 bytes).  Scripts
  with large integer vectors have 2× cache pressure.
- **Binary formats silently break**: pre-2c programs that write
  `f += scalar_integer` to BigEndian/LittleEndian files now
  produce 8-byte writes.  Every file format writer needs an `as
  <width>` cast per field.  The GLB writer was the flagship fix
  (`lib/graphics/src/glb.loft`).
- **Cross-crate FFI split**: `vector<integer>` elements stay i32
  (4 bytes) to match pre-compiled cdylib packages
  (`lib/graphics/native`, `lib/moros_render`).  In-process
  integers are i64.  Narrow→wide conversions happen at the
  boundary.  Do not "clean this up" without a coordinated
  cdylib rebuild.
- **Opcode footprint**: 26 duplicate opcodes still live (B above
  hasn't landed); interpreter dispatch table stays bloated until
  round 10b closes.
- **Narrow-int codegen surface**: `emit_field` needs
  `forced_size: Option<u8>` threaded in from `Attribute.alias_d_nr`.
  Any future codegen path that emits a field must do the same
  (or it'll land back in the D.d failure mode).

## Critical files touched by A / B / C

**A — hardening:**
- `src/state/codegen.rs` — size-consistency asserts at every
  `OpSet*` / `OpGet*` emission site.
- `lib/graphics/src/glb.loft` — already done (`74aefb4`), verify.
- Other binary writers under `lib/graphics/`, `lib/moros_render/`,
  `tests/` that use `f += scalar` in non-text format.

**B — Op*Long dedup:**
- `default/01_code.loft` — 26 `fn OpXxxLong` declarations.
- `src/ops.rs` — `op_xxx_long` function definitions (lines
  ~276-570 per `05-opcode-reclamation.md`).
- `src/fill.rs` — `OPERATORS[268]` table + the `fn op_xxx_long`
  dispatch thunks.
- `src/parser/operators.rs` — if any branch emits OpXxxLong
  directly.
- `src/state/codegen.rs` — any OpXxxLong codegen calls.

**C — Type::Long removal:**
- `src/data.rs::Type` enum + `I32` static + associated impls.
- `src/parser/definitions.rs:1068` — default integer range.
- `src/parser/mod.rs::convert` — Long ↔ Integer implicit
  conversion logic.
- `src/typedef.rs` — fill_database Long handling.
- `src/state/**` + `src/generation/**` — Type::Long match arms.
- `src/lexer.rs` — `long` keyword table.
- `doc/claude/LOFT.md` — `long` type documentation.

## Verification per phase

Every phase runs through the same gate:

```bash
./scripts/find_problems.sh --bg --wait   # 0 failures
cargo clippy --release -- -D warnings    # clean
cargo fmt --check                        # clean
cargo test --release --test wrap         # 47/47
cargo test --release --test native       # 5/5
```

Plus phase-specific:

- **A**: grep confirms zero `OpSet*` / `OpGet*` emission sites
  without the width assertion.
- **B**: `grep -r OpAddLong src/ lib/ default/` returns 0 results.
  `fill.rs` operator count decreases by 26.
- **C**: `grep -r "Type::Long" src/` returns 0 results.  Full
  suite passes without the variant.  Literal `9_000_000_000`
  type-checks as `integer`, not `long`.

## Closing the initiative

The initiative closes when:

1. **Code**: A + B + C land.  Opcode table is 242 (268 - 26).
   No `Type::Long` variant remains.
2. **Docs**: E lands.  `QUALITY.md § C54` moves to closed,
   `CHANGELOG.md` has the migration entry, `CAVEATS.md` documents
   the downsides.
3. **Migration**: D either ships or is explicitly deferred with
   a fresh-DB-start note in `RELEASE.md`.
4. **Gate**: `PROBLEMS.md` has no open C54 entries.
   `RELEASE.md` safety gate passes.

Total remaining effort: **8-14 hours** across 2-3 sessions.  Low
on drama — the hard decisions (G vs G', atomic 2+4 landing,
cross-backend parity) are all decided and shipped.

## Provenance

- Session log: this branch's commit history from `ae121cb`
  through `74aefb4` (11 fixes covering D.1 → all green).
- Baseline state: `PHASE_2C_PROGRESS.md` as of 2026-04-20.
- Strategic plan: `README.md` § phases 4-6.
- Tactical plans cited: `CATEGORY_C_PLAN.md` (Stage 3 assertion),
  `05-opcode-reclamation.md` (opcode inventory).
- Decisions: closed-by-decision entries are in
  `doc/claude/DESIGN_DECISIONS.md`.
