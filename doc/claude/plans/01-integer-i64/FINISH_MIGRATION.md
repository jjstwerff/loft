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
| 04 — deprecate `long` | **Done** — `Type::Long` variant, `long` keyword, and `l` literal suffix all removed (commits `3e976b3`..`0c46abb`). |
| 05 — opcode reclamation | **Done** — 34 `Op*Long` opcodes removed; OPERATORS table 268 → 234 (rounds 10b.1–10b.4 + 10d). |
| 06 — spec docs | **Done** — `CHANGELOG.md`, `LOFT.md`, `CAVEATS.md § C54`, this file. |

Remaining failure count: **0**.  Every initially-surfaced regression
(59 → 0) plus the follow-up hardening rounds in this session have
landed.  The initiative is shipping end-to-end.

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

**Status (2026-04-21):** **Done** — 34 `Op*Long` opcodes removed
across five commits.  OPERATORS table **268 → 234**.

| Round | Commit | Opcodes removed |
|---|---|---|
| 10b.1 | `5b2c89c` | `OpAbsLong` |
| 10b.2 | `fd09612` | `OpEqLong`, `OpNeLong`, `OpLtLong`, `OpLeLong` |
| 10b.3 | `cb0644c` | `OpMinSingleLong`, `OpConvFloatFromLong`, `OpConvBoolFromLong`, `OpAdd/Min/Mul/Div/Rem Long` + `Nullable` variants, `OpLand/Lor/Eor/SLeft/SRight Long` (18 ops) |
| 10d   | `e5a4988` | `OpConstLong`, `OpVarLong`, `OpPutLong`, `OpConvLongFromNull`, `OpCastIntFromLong`, `OpConvLongFromInt`, `OpCastLongFromText`, `OpCastLongFromSingle`, `OpCastLongFromFloat`, `OpGetLong` → `OpGetInt`, `OpSetLong` → `OpSetInt`; `OpFormatLong`/`OpFormatStackLong` renamed to `OpFormatInt`/`OpFormatStackInt` (11 slots reclaimed) |
| post  | `3b34f89` | Dead `OpXxxLong` match arms in `parser/operators.rs::rewrite_outer_arith_to_nullable` and `const_eval.rs::fold_op` cleaned up |

Parser emission sites rewired to the `Int` variants:
`state/codegen.rs::Value::Long → OpConstInt`,
`parser/operators.rs::call_to_set_op`'s f#next seek+set path,
`parser/objects.rs::file#index/#next`,
`parser/collections.rs::append_data_long` (suffix `"Long"` → `"Int"`),
`generation/dispatch.rs::format_long`,
`compile.rs::gather_const_literals` (OpSetLong dropped).

**Root cause of the earlier "OpSRightLong regression"**: the
`.loftc` bytecode cache key (`src/cache.rs:26`, `src/main.rs:1653`)
hashes only `(version, build_id, top-level script content)` — it
does NOT include `default/*.loft` or transitively-loaded library
content.  When `default/01_code.loft` is edited in-tree, the cache
key for `lib/moros_render/examples/moros_glb.loft` is unchanged,
so `byte_code_with_cache` loads bytecode compiled against the
*previous* opcode numbering.  The runtime then dispatches ops
through a renumbered `OPERATORS` table, producing silent data
corruption or SIGSEGV depending on how far the drift goes.  The
bisection that pointed at `OpSRightLong` was a false lead: any
deletion would have reproduced it, the cache just happened to
hold bytecode that still worked for the other single-op tests.

**Workaround** (required for anyone editing `default/*.loft`
between runs): delete cached `.loftc` files with
`find . -name '*.loftc' -not -path '*/target/*' -delete`.  The
underlying cache-invalidation bug is orthogonal to the migration
and tracked in **§ B.postscript**.

**Remaining `Long`-suffixed names** (intentional, not user-visible):
- `OpConstLongText` — unrelated ("const-long-text" = string from
  stream).
- `parallel_get_long` native FFI function — i64 signature distinct
  from `parallel_get_int` (i32) at the FFI layer.
- `FvLong` AST variant label in `lib/code.loft`.
- `store.get_long` / `set_long` Rust methods on the raw store.
- `op_*_long` helpers in `src/ops.rs` (internal; `op_*_int`
  forwards).
- `base_type("long", 8)` at database type index 1 (format
  stability — `keys.rs` / `state/io.rs` key-comparators hardcode
  this index).
**Risk**: low — each `OpXxxLong` body already delegates to the
i64 arithmetic.  The deduplication is a rename at call sites.

#### B.postscript — cache-key bug (separate follow-up)

`src/cache.rs::cache_key` at `src/main.rs:1653` is called with
`sources = [(abs_file, source_content)]` — a one-element list
holding only the top-level script.  When `default/*.loft` or a
transitively-loaded library under `lib/<pkg>/src/` changes
in-tree between runs of the *same* top-level script, the cache
key stays the same and `read_cache` returns stale bytecode
compiled against the previous stdlib opcode numbering.

This is OK during normal development (the git commit changes,
which bumps `LOFT_BUILD_ID` via `build.rs` and invalidates the
cache) but goes wrong during *uncommitted* stdlib edits — exactly
the loft-developer workflow.

Fix: extend `sources` at the `main.rs:1653` call site to include
every `default/*.loft` file and every `use`-imported lib module's
source content.  The parser already tracks which files were read
(it records `Position::file` in each diagnostic); thread that
list into `byte_code_with_cache`.  Alternative: weaken by
including just `default/*.loft` mtimes in the cache key — cheaper
but coarser.  Neither is in scope for the i64 migration; file as
a separate PROBLEMS.md entry.

### C — Round 10c: remove `Type::Long` and widen default range

**Status (2026-04-21):** **Done** across 5 commits.  The `long`
type name, the `l` literal suffix, and every user-facing `Long`
reference are gone from the loft source surface.  The Rust
`Type::Long` enum variant is deleted; 40 `Type::Long =>` match arms
across parser / codegen / scopes / variables / debug / native-FFI
paths were pruned as dead code.  A new `data::I64` constant
(`Type::Integer(i32::MIN+1, u32::MAX, false)`) is the wide-range
integer that sites which used to produce `Type::Long` now produce
instead.

| Commit | What landed |
|---|---|
| `3e976b3` | Parser stops emitting `Type::Long`: `long` keyword → I64, wide `integer limit(...)` → I64, `file#size/#index/#next` → I64, `LexItem::Long` literal → I64, iter-state vars → I64.  `Value::Long` is retained as the IR payload for i64 literals. |
| `44b525c` | `Type::Long` variant deleted from `data::Type`.  Match arms updated: native FFI type selection, `FieldValue` (`FvLong`) variant mapping, and debug readout now use `Type::Integer(_, max, _) if *max > i32::MAX as u32` to pick the i64 path. |
| `87ec05a` | Plan doc updated. |
| `11e1d47` | 37 `long` / `as long` / `-> long` / `long not null` / `<long>` references swept in `lib/*.loft`, `default/02_images.loft`, `default/04_stacktrace.loft`, `default/06_json.loft`, and `lib/graphics/src/graphics.loft` (color_a simplified). |
| `66c1146` | **Keyword removal** — `pub type long size(8);` deleted from `default/01_code.loft`; the `long` keyword intercept in `src/parser/definitions.rs` removed (writing `long` now fails with `"Undefined type long"`); the `l` literal suffix parsing in `src/lexer.rs::ret_number` dropped.  36 `Nl` literals swept across `bench/01..10`, stdlib, tests. |
| `0c46abb` | Final Rust-side cleanup: dead `"long"` arm in `typedef.rs::complete_definition`, `"long"` entry in `documentation.rs`'s built-in type list. |

**Kept intentionally (not user-visible):**

- `base_type("long", 8)` at runtime type index 1 — `keys.rs` and
  `state/io.rs` compare against hardcoded type indices, so the
  slot label stays for format stability.
- `kt_long` dispatch in `src/native.rs` JSON-to-struct populators
  — defensive fallback, unreachable for parser-produced types.
- `src/migrate_long.rs` — `loft --migrate-long <path>` CLI that
  rewrites external loft sources (`long` → `integer`, `42l` → `42`).

**Regression gate**: full `find_problems.sh --wait` green at
`0c46abb`.  Wrap + issues + parse_errors + native + exit_codes all
pass.  Clippy + fmt clean.

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

Historical execution order (all shipped except D):

1. **A1** (codegen assertions) — commit `358c155`.
2. **A2** (binary-format audit) — commit `74aefb4`.
3. **B** (Op*Long dedup) — commits `5b2c89c`, `fd09612`, `cb0644c`,
   `e5a4988`, `3b34f89`.  34 opcodes reclaimed.
4. **C** (Type::Long removal) — commits `3e976b3`..`0c46abb`.
5. **E** (docs) — this commit.
6. **D** (migration tool) — deferred.  No known user with a
   persisted database; document the incompatibility in
   `RELEASE.md` and require fresh-DB start when an external
   user first hits it.

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
