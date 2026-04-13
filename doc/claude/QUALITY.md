# QUALITY — Open Issues, Active Designs, Enhancement Plan

This document is the single source of truth for **what's broken, what's
being fixed, and what should be fixed next**.  It replaces the earlier
BITING_PLAN.md (which mixed status, design, and history) and
consolidates the open-issue tracking that previously drifted between
PROBLEMS.md and CAVEATS.md.

Read order:
1. § Open programmer-biting issues — the live work queue
2. § Active sprint — P54 — current focus, with steps remaining
3. § Active design — C54 — the next big landing
4. § Compiler blockers — struct-enum bugs (B2…B7) gating P54 + future enums
5. § Enhancement tiers — quality investments ranked by leverage

History and closed items live in [CHANGELOG.md](../../CHANGELOG.md).
Decisions to *not* fix something live in
[DESIGN_DECISIONS.md](DESIGN_DECISIONS.md).

---

## Open programmer-biting issues

| # | Issue | Severity | Status |
|---|-------|----------|--------|
| P54 | `json_items` returns opaque `vector<text>`; `MyStruct.parse(text)` silently zeroes on malformed input | High | **Active sprint** — see § P54 below |
| Q1 | `json_errors()` reports byte offset only — no path, no line:column, no context snippet | Medium | **Designed, not landed** — see § Q1 below |
| Q2 | No free-form object iteration / key listing / quick `kind(v)` peek | Medium | **Designed, not landed** — see § Q2 below |
| Q3 | No `to_json(v)` serialiser — reads but can't write or round-trip | Medium | **Designed, not landed** — see § Q3 below |
| Q4 | No way to construct `JsonValue` trees in loft code (fixtures, mocking, forwarding) | Medium | **Designed, not landed** — see § Q4 below |
| C54 | `integer` arithmetic on `i32::MIN` silently returns null | Medium | **Designed, not landed** — see § C54 below |
| B2-runtime | Unit-variant literal construction in struct-enum crashes | Medium | Compiler — see § Compiler blockers |
| B3 | Struct-enum tail-expression return crashes | Medium | Compiler — see § Compiler blockers |
| B5 | Recursive struct-enum trips codegen recursion guard | Medium | Compiler — workaround: arena indirection |
| B7 | Native-returned struct-enum temporaries leak intermediate stores | Medium | Compiler — unblocks 5 P54 ignored tests |

Items that look open in the historical sections of PROBLEMS.md /
CAVEATS.md but are now closed: P22, P91, P135 / C58, P137, P139, C60,
INC#3, INC#29.  See CHANGELOG.md.

---

## Active sprint — P54 (`JsonValue` enum)

**Bite.** `MyStruct.parse(text)` silently returns a zero-valued struct
on malformed JSON — no type check, no runtime diagnostic — contradicting
loft's "static types catch mistakes" promise.

**Decision.** Replace the text-based JSON surface with a first-class
`JsonValue` enum.  `json_parse(text) -> JsonValue` is the one entry
point; `MyStruct.parse(JsonValue)` accepts only the typed tree; the
old `json_items` / `json_nested` / `json_long` / `json_float` /
`json_bool` family is withdrawn.

### Surface (`default/06_json.loft`)

```loft
pub enum JsonValue {
    JNull,
    JBool   { value: boolean },
    JNumber { value: float not null },
    JString { value: text },
    JArray  { items_id: integer },     // arena index — see § B5 workaround
    JObject { fields_id: integer },
}

pub fn json_parse(raw: text) -> JsonValue;
pub fn json_errors() -> text;
pub fn field(self: JsonValue, name: text) -> JsonValue;
pub fn item(self: JsonValue, index: integer) -> JsonValue;
pub fn len(self: JsonValue) -> integer;
pub fn as_text(self: JsonValue) -> text;
pub fn as_number(self: JsonValue) -> float;
pub fn as_long(self: JsonValue) -> long;
pub fn as_bool(self: JsonValue) -> boolean;
```

Pattern matching falls out of existing struct-enum machinery:

```loft
match json_parse(raw) {
    JObject { fields_id } => { … },
    JArray  { items_id }  => { … },
    JNull                 => println("parse error: {json_errors()}"),
    _                     => println("unexpected root kind"),
}
```

### Status (2026-04-13)

| Layer | State |
|---|---|
| Stdlib enum + surface signatures | **Shipped** (`default/06_json.loft`) |
| Rust JSON parser (`src/json.rs`) | **Shipped** — full RFC 8259, 9 unit tests |
| `n_json_parse` for primitives (null/bool/number/string) | **Shipped** |
| `n_json_errors` | **Shipped** |
| `n_as_text`, `n_as_number`, `n_as_long`, `n_as_bool`, `n_field`, `n_item`, `n_len` | **Shipped** — primitives real, indexers stubbed (return JNull) |
| Acceptance tests | **26 green, 13 ignored** in `tests/issues.rs::p54_*` |

### Remaining steps

**Step 4 (arena materialisation).**  Make `JArray` / `JObject` real.
The recursive enum form `JArray { items: vector<JsonValue> }` trips
B5.  Workaround: arena indirection — children are stored in a per-parse
allocation and referenced by integer index (`items_id`, `fields_id`).
The arena is allocated in the **same store** as the root JsonValue so
the entire tree frees as one unit when the root DbRef goes out of
scope (the `File` pattern, not `stores.database()`).  Critical files:
`src/native.rs::n_json_parse` (extend with `write_value_tree` walking
the `Parsed` tree from `src/json.rs`); `src/native.rs::n_field` /
`n_item` / `n_len` (real implementations dispatching on discriminant).

**Step 5 (`Type::parse(JsonValue)` codegen).**  Per-struct unwrap that
walks the schema, calls `n_field` for each declared field, converts
via the `n_as_*` extractors, stores into the destination.  Site:
`src/parser/objects.rs:527` (`parse_type_parse`).

**Field-type matrix** (explicit policy — the P54 bite was silent
field-level zeroing; this spells out the replacement):

| Declared field type | JSON produces | Target value |
|---|---|---|
| `text` | `JString`        | value |
| `text` | anything else    | null text + diagnostic |
| `integer` / `long` | `JNumber` (integral) | value |
| `integer` / `long` | `JNumber` (fractional) | null + diagnostic (lossy cast) |
| `float` | `JNumber` | value |
| `boolean` | `JBool` | value |
| `T` (nested struct) | `JObject` | recurse `T.parse(subtree)` |
| `vector<T>` | `JArray` | iterate + `T.parse` each element |
| `JsonValue` (explicit typing) | any kind | capture the subtree verbatim — the hybrid case, lets typed ingestion coexist with deferred free-form inspection |
| any | `JNull` | declared default |
| any | missing field | declared default |

**Strict vs. permissive** (opt-in per call):

```loft
u = User.parse(v);                  // permissive (default)
u = User.parse(v, strict: true);    // rejects on any deviation
```

- **Permissive** (default): missing fields, extra fields, and
  type-mismatch leaves keep the declared default.  Every deviation
  appends an entry to `json_errors()` so users can opt in to
  diagnostics even without `strict`.  This matches how loft's
  `null`-sentinel discipline is used elsewhere — absence is not
  failure.
- **Strict**: first deviation returns `null` at the top-level
  `parse` call, and `json_errors()` contains the full list of
  deviations with their paths (via Q1 infrastructure).

**Diagnostic shape** (Q1 path + line:column extend to schema errors):

```
User.parse error at /users/3/age (byte 12847, line 423 col 20):
  expected integer, got JString "thirty"
```

`vector<T>.parse(v)` — when a top-level array maps to a homogeneous
vector of T, the same machinery applies per-element.  Each
mismatched element appends a path `/N` diagnostic.

**Root-shape rules**:
- `T.parse(v)` where `v` is not `JObject` → returns `null`, logs
  `"expected JObject at /, got JArray"`.
- `vector<T>.parse(v)` where `v` is not `JArray` → returns an empty
  vector, logs `"expected JArray at /, got JObject"`.

**Step 6 (gate `MyStruct.parse(text)`).**  Same parser site.  If the
argument type is `Type::Text(_)` and the target is a struct, emit
`"MyStruct.parse expects a JsonValue, got text — call json_parse(text)
first"`.  Migration blocked: `tests/scripts/57-json.loft` and
`tests/docs/24-json.loft` have ~20 legitimate `Struct.parse(text)`
sites that must be rewritten first.

**Step 7 (unignore acceptance tests).**  13 `#[ignore]`'d in
`tests/issues.rs::p54_*`.  Each goes green automatically as the
corresponding layer lands.  Five of those — the text-return-through-fn
family + chained-access — depend on **B7** in § Compiler blockers
below; one fix unblocks all five.

**Step 8 (docs).**  LOFT.md JSON section in pattern-matching chapter;
STDLIB.md JSON chapter; CHANGELOG entry.

### Acceptance

`cargo test --release --test issues p54_` — all 39+ tests green.
Brick Buster / Moros editor read JSON via the new surface.  No call
site in `default/`, `lib/`, or `tests/` uses `Struct.parse(text)`.

---

## Active design — C54 (integer i64)

**Bite.** Any arithmetic landing on `i32::MIN` silently returns null
(and debug-aborts).  Division by zero same.  In a language pitched as
"reads like Python" this is hostile — users hit it multiplying
microsecond timestamps, accumulating checksums, building bitmasks.

### Decision — decouple arithmetic width from storage width

- **Arithmetic is always i64.**  `i64::MIN` is the null sentinel;
  reaching it by accident is astronomically unlikely.
- **Storage width unchanged for bounded fields.**  `integer
  limit(0, 255)` still takes 1 byte.  `u8` / `u16` / `i8` / `i16`
  unchanged.
- **Unbounded `integer` stores as i64 (8 bytes).**  Users wanting the
  old 4-byte default write `i32` (existing alias for `integer
  size(4)`).
- **Load-widens, store-narrows.**  Reading any bounded integer field
  widens to i64 in registers; arithmetic at i64; writing back narrows
  with the existing `limit` range check.

### Sub-tickets (sprint branch `c54-integer-i64`)

**C54.A — Widen `integer` to i64 + range-packed storage.**  Replumb
`Op*Int` opcodes on i64 registers.  Flip `Type::size()` default arm
4 → 8 for unbounded.  `get_int` / `set_int` widen-on-load,
narrow-on-store.  Bump `.loftc` cache version.  Ship
`loft --migrate-i64 <dbfile>` for persisted databases.

The bytecode constant family stays width-graded by magnitude:

| Opcode | Stream bytes | Range |
|---|---|---|
| `OpConstTiny`  | 1 | −128 ..= 127 |
| `OpConstShort` | 2 | −32 768 ..= 32 767 |
| `OpConstInt`   | 4 | −2³¹ ..= 2³¹ − 1 |
| `OpConstLong`  | 8 | full i64 |

Each sign-extends into the i64 register on load.  The common case
(`x = 0`, loop bounds, array indices) stores 1 byte after the opcode
— ~50 % bytecode-size saving on integer-heavy code.

Tests (all `#[ignore]`'d initially):
- `c54_i32_min_round_trip` — `-2_147_483_648 * 1` returns the same value, not null.
- `c54_arithmetic_at_boundary`, `c54_bounded_storage_preserved`,
  `c54_unbounded_storage_widens`, `c54_u8_times_u8_no_overflow`,
  `c54_loftc_cache_invalidated`, `c54_migration_tool_roundtrip`.
- `c54a_const_tiny_used_for_small_literals` — guard the width-graded
  encoding against well-meaning flatten attempts.

**C54.C — Add `u32` as a stdlib type.**  Post-A, `u32` is trivially
expressible:

```loft
pub type u32 = integer limit(0, 4_294_967_294) size(4);
```

The sentinel reservation (one short of 2³²) matches `u8 = integer
limit(0, 255) size(1)`.  Users needing the exact top value write
`u32 not null`.  Closes the "RGBA pixels wrap negative" trap.
Tests: `c54c_u32_rgba_round_trip`, `c54c_u32_arithmetic_promotes`,
`c54c_u32_not_null_full_range`, `c54c_u32_size_is_4`.

**C54.B — Remove `long` + `l` literal suffix (deprecate 0.9.0,
remove 1.0.0).**  Once `integer` is i64, `long` is a redundant alias
and `10l` is meaningless.  Ship `loft --migrate-long <path>` to
rewrite user code.  Repo migration is **immediate**: stdlib /
tests / lib all use `integer` / plain literals on the C54.B branch
to avoid a second sweep at 1.0.0.  Tests: `c54b_long_type_deprecated`,
`c54b_l_literal_deprecated`, `c54b_long_migration_tool`,
`c54b_stdlib_no_long`.

**C54.E — Free the 32-bit-arithmetic opcodes.**  After A, every
`integer` slot is i64; the `Op*Long` arithmetic family becomes
duplicate.  Delete `OpAddLong`, `OpMulLong`, `OpEqLong`, … from
`default/01_code.loft` and `src/native.rs`'s registry.  Reclaim ~26
opcode slots out of the current 254/256-of-256 budget.  This unblocks
O1 superinstruction peephole rewriting (see ROADMAP.md § Deferred
indefinitely).  **Keep** the bytecode-constant family
(`OpConstTiny`/`Short`/`Int`/`Long`) — those are stream-payload-width
optimisations, not register-width specific.  Tests:
`c54e_opcode_budget_reclaimed`, `c54e_long_arithmetic_still_works`,
`c54e_loftc_pre_c54_invalidated`.

**C54.D — Rust-style literal suffixes.**  Closed by decision
([DESIGN_DECISIONS.md § C54.D](DESIGN_DECISIONS.md#c54d--rust-style-numeric-literal-suffixes)).

### Ordering

1. **C54.A** — runtime/schema widening (must land first).
2. **C54.C** — `u32` type (depends on A's narrow-store machinery).
3. **C54.B** — sweep stdlib/tests, deprecation warnings for users.
4. **C54.E** — delete duplicate opcodes (requires B's sweep first or
   build cascades).

### Migration cheat-sheet for users

| Old code | After C54 | Action |
|---|---|---|
| `x: integer` | 8-byte storage, i64 arithmetic | Add `limit(...)` if compact storage matters |
| `x: long` | deprecated; alias for `integer` | `loft --migrate-long` |
| `x = 10l;` | deprecated; use `x = 10;` | `loft --migrate-long` |
| `x: u8`/`u16`/`i8`/`i16` | unchanged | None |
| `x: u32` | **new** — 4-byte storage, full u32 range minus sentinel | Opt in where applicable |
| `x: i32` | alias for `integer size(4)` — opts *into* classic 32-bit range | None; MIN trap is opt-in |
| `0xAABBCCDD` stored as integer | silently a negative i32 | Declare as `u32` — stores clean |

### What this design is not

- Not arbitrary precision — fixed-width i64.
- Not removal of the null sentinel — `i64::MIN` still represents
  null; reaching it by accident becomes astronomically unlikely.
- Not a schema rewrite for bounded fields.
- Not Rust-style literal suffixes.

---

## Active design — Q1 (JSON parse-error diagnostics)

**Bite.** `json_errors()` today returns `"{msg} (byte {at})"` — a
human-readable message plus the raw byte offset into the source.  For
a 50 KB configuration file or an API response, this is effectively
unusable: users can't tell *which field* failed, what line:column to
open the file at, or what the surrounding JSON looks like.  The whole
P54 pitch is "typed tree catches what `Struct.parse(text)` used to
silently swallow" — that win is half-delivered if the diagnostic on
failure is `byte 12847`.

**Status (2026-04-13).**  `src/json.rs::parse` currently returns
`Result<Parsed, (String, usize)>`.  `n_json_parse` formats the tuple
verbatim into `stores.last_json_errors`.  The parser stops at the
first failure (the `Vec<String>` field can hold many but only one is
ever written).

### Target diagnostic

```
parse error at line 423 col 17 (byte 12847):
  path: /users/3/address/zip
  expected digit after `.`
    421 │       {
    422 │         "address": {
    423 │           "zip": 1.}
                          ^
    424 │         }
```

Three pieces, each independently useful:

1. **JSON Pointer path (RFC 6901).**  `/users/3/address/zip` — names
   the field.  Accumulated during descent: push `/users` entering
   that object's field, push `/3` entering the array element, …  On
   error, the current path is the location.  Storage: `Vec<String>`
   in the parser; push on descent, pop on ascent.

2. **Line:column.**  One pass over `bytes[0..offset]` counting `\n`
   converts the byte offset at error time.  O(n) but only executed
   on failure, not per token.

3. **Context snippet.**  Two lines before, the error line with a
   caret under the offending byte, one line after.  Trivial once
   line:column is known.

### Surface changes

**`src/json.rs`:**

```rust
pub struct ParseError {
    pub message: String,
    pub byte_offset: usize,
    pub path: String,        // RFC 6901 pointer; "" for root
}

pub fn parse(input: &str) -> Result<Parsed, ParseError>;
```

Internal parser functions gain a `&mut Vec<String>` path stack.
`parse_object` pushes `/escape(name)` before recursing on each
field's value, pops after.  `parse_array` pushes `/{index}` and
pops the same way.  Push/pop is O(1); no extra allocation per
token.

RFC 6901 escaping: `~` → `~0`, `/` → `~1`.  Five-line helper.

**`src/native.rs::n_json_parse`:**

```rust
Err(ParseError { message, byte_offset, path }) => {
    let (line, col) = line_col_of(raw.as_bytes(), byte_offset);
    let snippet = context_snippet(raw, byte_offset, 2, 1);  // 2 before, 1 after
    stores.last_json_errors.clear();
    stores.last_json_errors.push(format!(
        "parse error at line {line} col {col} (byte {byte_offset}):\n\
         \x20 path: {path}\n\
         \x20 {message}\n\
         {snippet}"
    ));
}
```

Multiple errors: keep `Vec<String>` shape; future step (not this
landing) can teach the parser to continue past recoverable errors
— `json_errors()` would then return one line per failure.  For
today's single-error-at-first-fail parser, the Vec holds one well-
formatted entry.

**`default/06_json.loft`:**
No change — `json_errors()` signature (`-> text`) is already the
right shape.  What callers *see* in that text becomes useful.

### Implementation cost

~60 lines in `src/json.rs` (`ParseError` struct, path-stack plumbing
in 6 parse functions, RFC 6901 escape helper, line:column converter,
context-window formatter).  ~20 lines in `n_json_parse` to replace
the tuple-destructure with the rich format.

### Tests

All additions to `tests/issues.rs::p54_*` — already the right file:

- `p54_err_reports_path_into_nested_object` — parse a malformed
  `{"a": {"b": 1.}}` and assert `json_errors()` contains `/a/b`.
- `p54_err_reports_path_into_array_element` — `[1, 2, 1.]` contains
  `/2`.
- `p54_err_reports_line_and_column` — `"{\n  \"x\": 1.\n}"` reports
  line 2.
- `p54_err_context_snippet_includes_caret` — the snippet block has
  a `^` under the offending column.
- `p54_err_path_escapes_slash_and_tilde` — a field named `a/b~c`
  renders as `/a~1b~0c`.
- `src/json.rs` unit tests gain a `path_in_error` case so the
  parser-side guarantee is covered even without the native wrapper.

### Why Tier 2 (not Tier 1)

This doesn't unblock any ignored test and doesn't close a crash.
It's an *ergonomics* win that substantially improves the P54 value
proposition.  Landing it inside the P54 sprint — between step 5
(`Type::parse(JsonValue)`) and step 6 (`.parse(text)` rejection
diagnostic) — is natural: step 6 will want to print a useful
diagnostic when users pass text, and that diagnostic can reuse the
line:column + context-snippet helper.

### Schema-side reuse (P54 step 5)

`Type::parse(JsonValue)` generates its own deviations (missing
required field, type mismatch at a leaf, wrong root kind).  These
reuse the same path + line:column + snippet infrastructure:

```
User.parse error at /address/zip (byte 2047, line 48 col 20):
  expected integer, got JString "10012"
```

Implementation: schema codegen passes its current path (struct
field name or `/N` for vector elements) into the same formatter
used by the parser.  No second diagnostic system.

### What this design is not

- Not a JSON Schema validator — the diagnostic reports *where* the
  parser or schema-walker gave up, not *what a user's business
  rules* expected.
- Not a recovering parser — first parser error still stops.  A
  recovering mode is a follow-up with its own design trade-offs.

---

## Active design — Q2 (free-form object iteration + kind peek)

**Bite.** A user holding a `JsonValue` of unknown shape has no way
to list an object's keys or iterate its fields.  `JObject {
fields_id }` exposes an arena index, not something loopable.
Without this, "free-form" reduces to "guess candidate key names
and try `field()` on each" — which isn't free-form at all.

`match`'s seven-arm dispatch also isn't great for a one-line
"what kind did I get?" peek in logs or conditional branches.

### Surface

```loft
/// Returns the variant name as text: "JNull", "JBool",
/// "JNumber", "JString", "JArray", "JObject".  Cheap — reads the
/// discriminant byte, formats a literal.
pub fn kind(self: JsonValue) -> text;

/// JObject: returns the vector of declared field names in
/// insertion order.  Any other variant: empty vector.
pub fn keys(self: JsonValue) -> vector<text>;

/// JObject: returns the vector of (name, value) entries so a
/// user can `for entry in fields(v) { … entry.name … entry.value … }`.
/// Any other variant: empty vector.
pub fn fields(self: JsonValue) -> vector<JsonField>;

/// JObject: true if the key is present (even if its value is JNull).
/// Distinguishes "absent" from "present-but-null".
pub fn has_field(self: JsonValue, name: text) -> boolean;
```

`JsonField` already exists in the stdlib for schema-internal use;
this promotes it to the public surface.

### Implementation

- `n_kind` — 10 lines; reads the discriminant, writes a static
  string literal via `stores.scratch`.
- `n_keys` / `n_fields` / `n_has_field` — dispatch on
  discriminant, read the arena's object record, walk its
  `JsonField` vector.  Same arena machinery P54 step 4 builds.

### Iteration example

```loft
v = json_parse(raw);
match v {
    JObject { fields_id } => {
        for entry in fields(v) {
            println("{entry.name}: {kind(entry.value)}");
        }
    }
    _ => println("not an object"),
}
```

### Tests

- `q2_kind_reports_each_variant` — one assertion per variant.
- `q2_keys_preserves_insertion_order` — `{"b":1, "a":2}` → `["b", "a"]`.
- `q2_fields_iteration` — walk all entries of a three-field object.
- `q2_has_field_distinguishes_absent_from_null` —
  `{"a": null, "b": 1}` → `has_field("a")=true`, `has_field("c")=false`.
- `q2_kind_of_nested_walk` — kind() works on intermediate
  `field()` results.

### Depends on

P54 step 4 (arena materialisation).  Lands immediately after.

---

## Active design — Q3 (`to_json` serialiser + struct serialisation)

**Bite.** The current surface is read-only.  Users who parse a
JSON response, modify a subtree, and want to forward it — or
users building a JSON reply from a loft struct — have no way to
emit JSON text.  Round-trip testing (parse → compare →
serialise → compare) is impossible.

### Surface

```loft
/// Serialise a JsonValue tree to canonical JSON text.
/// Object keys emitted in insertion order; no extraneous
/// whitespace; numbers formatted per RFC 8259.
pub fn to_json(self: JsonValue) -> text;

/// Pretty-printed variant — 2-space indent, one element per line
/// for arrays/objects with >1 element.  Useful for logs and
/// golden-file tests.
pub fn to_json_pretty(self: JsonValue) -> text;

/// Struct serialisation — inverse of `T.parse(JsonValue)`.
/// Walks the struct's schema, builds a JObject, recurses into
/// nested struct / vector fields.  Fields with null sentinel
/// values serialise as JSON null (or are omitted under
/// `skip_null: true`).
pub fn to_json(self: T) -> text;                  // one per type; codegen-generated
pub fn to_json_pretty(self: T) -> text;
```

### Field-type matrix for struct → JSON

| Field type | Serialisation |
|---|---|
| `text` | `JString` |
| `integer` / `long` | `JNumber` (integral) |
| `float` | `JNumber`; `NaN` / `inf` → JSON `null` + diagnostic |
| `boolean` | `JBool` |
| `T` (nested struct) | `JObject` (recurse) |
| `vector<T>` | `JArray` (iterate) |
| `JsonValue` | serialised verbatim (round-trip the captured subtree) |
| null sentinel | `null` by default; configurable |

### Canonical form

- **No whitespace** outside strings (pretty-printed form adds it
  back).
- **Numbers** use shortest round-trip representation (same as
  `{f}` formatter).
- **Strings** escape `"`, `\\`, and control bytes `< 0x20`; UTF-8
  bytes pass through verbatim (no `\uXXXX` escaping of BMP
  characters — RFC 8259 allows both; shortest wins).
- **Object key order** — insertion order for `to_json(JsonValue)`,
  declaration order for `to_json(T)`.  Not sorted — stable
  insertion order is useful for diffing and avoids surprise
  reordering when programs read-modify-write.

### Implementation

- `src/json.rs` gains `pub fn format(v: &Parsed, pretty: bool) ->
  String` — recursive walk writing into a `String` buffer.
- `n_to_json` — reads a `JsonValue` DbRef, walks the arena into a
  `Parsed`-shaped temporary, formats.  Or format directly from
  the arena representation; same cost.
- `T.to_json()` codegen at the struct-method generation site —
  walks the schema, emits `n_build_json_field` calls per field
  into a work-buffer arena, then formats.  Mirror image of step 5.

### Round-trip property

`parse(to_json(v)) == v` for every `JsonValue`.  Property test
asserts this on a generated corpus (null, booleans, numbers
including 0.1-family, unicode strings, nested up to depth 5).

### Tests

- `q3_primitives_round_trip` — each primitive variant.
- `q3_nested_object_round_trip`.
- `q3_array_of_mixed_kinds_round_trip`.
- `q3_pretty_form_valid_json` — `parse(to_json_pretty(v)) == v`.
- `q3_unicode_string_escaping` — `"α β 😊"` round-trips without
  `\uXXXX` escaping.
- `q3_struct_to_json` — `User { name: "Bob", age: 30 }.to_json()`
  produces `{"name":"Bob","age":30}`.
- `q3_struct_with_nested` — recurses into `Address`.
- `q3_struct_with_jsonvalue_field` — raw subtree forwards
  verbatim.
- `q3_null_float_becomes_json_null`.

### Depends on

P54 step 4 for the `JsonValue` serialisation side.  `T.to_json()`
lands after step 5 (same codegen machinery in reverse).

---

## Active design — Q4 (JsonValue construction in loft code)

**Bite.** Today a loft program can read a `JsonValue` but cannot
build one.  Test fixtures ("given this JSON, when I call my
function…"), reply-construction in a web service, and forwarding
synthesised payloads are all impossible.

The obvious syntax — `v = JString { value: "hi" }` — trips
**B2-runtime** (unit-variant / struct-enum literal construction
at runtime crashes).  Waiting for B2-runtime blocks Q4 on
multi-session compiler surgery.

### Surface — helper constructors (bypass B2-runtime)

```loft
pub fn json_null() -> JsonValue;
pub fn json_bool(v: boolean) -> JsonValue;
pub fn json_number(v: float not null) -> JsonValue;
pub fn json_string(v: text) -> JsonValue;
pub fn json_array(items: vector<JsonValue>) -> JsonValue;
pub fn json_object(fields: vector<JsonField>) -> JsonValue;
```

Plus a struct-literal shortcut for JsonField:

```loft
f = JsonField { name: "age", value: json_number(30.0) };
```

These are **native** functions that allocate arena records
directly — the same path `n_json_parse` uses internally.  They
sidestep B2-runtime because the variant is constructed in Rust,
not via loft's struct-enum literal syntax.

### Builder ergonomics

For object-heavy construction, a vector-of-fields literal reads
cleanly:

```loft
reply = json_object([
    JsonField { name: "status", value: json_string("ok") },
    JsonField { name: "count",  value: json_number(42.0) },
    JsonField { name: "data",   value: forwarded_subtree },
]);
```

If usage patterns show this is too verbose, a second-round API
(`json_object_of([("status", "ok"), ("count", 42)])` with inferred
variants) can land; deferred until real call sites exist.

### Mutation — deferred

Mutating an existing tree (`v.set_field(name, value)`,
`v.push_item(item)`, `v.remove_field(name)`) is a natural
follow-up but **not in scope** for Q4.  Reason: arena indirection
+ the current `OpFreeRef` discipline make in-place mutation of a
tree's children expensive to reason about.  The construction
helpers above let users build a new tree from parts; replacing a
subtree in a parsed tree can be done by constructing the new
object and handing it to the consumer.

### Tests

- `q4_build_primitives` — one test per constructor.
- `q4_build_array_round_trip` — `to_json(json_array([…]))` matches
  expected text.
- `q4_build_object_round_trip` — same for objects.
- `q4_nested_construction` — object containing an array of
  objects.
- `q4_fixture_for_parse` — build a tree, hand to
  `User.parse(v)`, assert the resulting struct.
- `q4_forward_captured_subtree` — parse → extract `JsonValue`
  field → embed in a new object → serialise.

### Depends on

P54 step 4 (arena machinery).  Q3's serialiser closes the
round-trip test surface but isn't strictly required — Q4's
constructors can land first.

### Why this belongs in P54 scope

Without Q4, P54 ships a one-way JSON pipeline.  Users can *read*
structured data but can't *write* it — so a loft web service
answering a request with JSON, a test that wants to mock a
response body, or any system that composes JSON from loft values
hits a wall.  "General-purpose JSON support" is the explicit P54
goal; Q4 is required for that, not an extra.

---

## Compiler blockers — struct-enum bugs

These bugs each surface any time a user writes a `Result<T, E>`-style
struct-enum, not just for JSON.  Fixing them unblocks the whole
`Option<T>` / `Result<T, E>` / planned coroutine-yield surfaces.

**B1 — Unit-variant match index-OOB.**  **FIXED** commit `61c36d7`.
Regression: `p54_b1_unit_variant_match_from_binding`.

**B2-runtime — Unit-variant literal construction in struct-enum
crashes.**  `JsonValue.JNull { is_null: true }` constructed at
runtime in a mixed enum doesn't produce a matchable value.
Workaround: build via the constructor path the parser uses; user
code avoids unit variants.  Test: `p54_b2_runtime_*` (`#[ignore]`).

**B3 — Struct-enum tail-expression return crashes.**  Five
investigation sessions narrowed the diagnosis: needs **at least
4 coordinated codegen layers** changed (caller-side hidden-slot
allocation, `scopes.rs:307-318` hoist, `OpCopyRecord` deep-copy paths,
OpReturn discard accounting).  Single or even 3-layer attempts mutate
the symptom but never close it.  Workaround: explicit `return n;`
instead of `n` at function tail.  Tests: `p54_b3_*` (`#[ignore]`).
Estimated 8-12 source-line ranges across 2 files when attempted as
one focused refactor.

**B5 — Recursive struct-enum infinite codegen loop.**  Declaring
`JArray { items: vector<JsonValue> }` trips the
`Recursion depth limit exceeded (500)` guard.  Workaround:
arena indirection (P54 step 0).  Test: `p54_b5_recursive_struct_enum`
(`#[ignore]`).

**B6 — Match-arm type unification.**  **FIXED** commit `5684df2`.
Regression: `p54_b6_match_arm_value_text_unifies`.

**B7 — Native-returned temporary lifecycle (broader than initially scoped).**

Scope analysis (`src/scopes.rs`) doesn't emit `OpFreeRef` correctly
for the JsonValue store returned by `json_parse`.  The store leaks
on a chain of method calls AND any subsequent method-call site
trips a double-free at exit even when the method does no
allocation of its own.  Confirmed symptoms:

- `n_json_parse` returning a string variant + `as_text()` →
  caller's text-return path frees the JsonValue store before the
  text copy completes (`free(): invalid next size` at exit).
- Chained JSON access (`v.field("a").item(0).field("b")`) leaks
  intermediate stores.
- `fn f() -> text { c = txt[0]; "{c}" }` SIGSEGVs (discovered
  while writing INC#9 regression tests) — same family:
  native-returned text temporary built via `n_format_text` on a
  character isn't tracked for free on the outer function's
  return path.
- **(new, found 2026-04-13)** ANY method call on a JsonValue
  local crashes — even a method that just reads the discriminant
  byte and returns an integer (`len(v)`).  The crash is exit-time
  double-free, but the test harness sees it as SIGSEGV before
  reporting the function's return value.  Discovered while
  attempting to ship Q2's `kind(v) -> integer` peek; reverted
  the ship and parked the regression guard at
  `b7_method_on_jsonvalue_returning_integer_crashes` (`#[ignore]`).
- A second `json_parse` call in the same function silently
  corrupts memory, even when the first result is assigned to a
  different local — the lifecycle bug compounds across calls.

**Blast radius**: the entire `(JsonValue) -> T` method surface is
gated on this fix, not just text returns.  This means **Q2**
(`kind`, `keys`, `fields`, `has_field`), **Q3** (`to_json`,
`to_json_pretty`), the planned step-4 implementations of
`field`/`item`/`len`, and parts of step 5 (`Type::parse(JsonValue)`)
all sit downstream.

**One fix unblocks 5 of the 13 ignored P54 tests** plus the new
`b7_method_*` guard, closes the INC#9 character-interpolation
crash, and clears the runway for Q2/Q3/Q4 + P54 step 4-5 to
land cleanly.  Highest-leverage compiler bite remaining — and
the bottleneck for nearly every JSON deliverable on the
roadmap.

---

## Enhancement tiers

Quality investments ranked by leverage.  Pick **one Tier 1** as the
multi-session sprint, pair with **one Tier 2** as a
session-of-the-week background bite.

### Tier 1 — closes whole classes of bugs

1. **B7 lifecycle for native-returned struct-enum temporaries.**
   Unblocks 5 P54 ignored tests in one fix.  Scope analysis pattern,
   precedent in `File`'s ref-count handling.

2. **C54 integer → i64.**  Eliminates the `i32::MIN` sentinel trap
   that has spawned three documented gotchas.  Multi-session,
   sub-tickets land independently (see § C54).

3. **Drive `#[ignore]`'d tests to zero.**  23+ in `tests/issues.rs`
   are the most honest backlog the project has.  Sustainable cadence:
   1–3 per session.

### Tier 2 — preventive, low-risk, high-readability

4. **`cargo clippy --no-default-features --all-targets -- -D warnings`
   cleanup.**  Currently fails on 12 issues in `parallel.rs`
   (single-char names, raw-pointer/safety, indexing patterns).
   Real lints, not false positives.  Hides future regressions
   because no one runs the gate clean today.  Ship as a permanent CI
   guard after.

5. **Migrate `Struct.parse(text)` → `json_parse(text) → match`** in
   `tests/scripts/57-json.loft` and `tests/docs/24-json.loft` once
   P54 step 5 lands.  Unblocks step 6 (the rejection diagnostic)
   and turns the tests into examples of the modern API.

6. **Document one inconsistency per session.**  INC#12 and INC#26
   are pure-doc bites following the INC#3 / INC#29 pattern — write
   the gotcha into LOFT.md, lock the behaviour with 2-3 regression
   tests.

### Tier 3 — structural, larger payoff

7. **Bytecode cache verification.**  `.loftc` shipped in commit
   `4039490`; no test exercises hit / miss / invalidation today.
   Single integration test: compile, cache, mutate source,
   recompile, assert re-codegen.  Catches staleness bugs before
   users do.

8. **Const store mmap path on Linux.**  Designed in CONST_STORE.md,
   partially shipped.  Measure startup wins on a real loft program
   (Brick Buster, Moros editor); lock as a benchmark to prevent
   regressions and surface value to users.

9. **WASM FS bridge.**  `tests/issues.rs` SIGSEGV-on-missing-file is
   blocked on the WASM virtual-FS work in WASM.md.  Single-session
   bites: stub `file().exists()` and `file().content()` to safe
   defaults under `wasm32`, ship a regression test.  Each loft web
   demo wanting file I/O depends on this.

### Tier 4 — process / hygiene

10. **PROBLEMS.md Quick-Reference is the source of truth — keep it
    that way.**  Three docs (Quick-Reference, long-form section,
    CAVEATS.md) drift independently and required two
    "doc hygiene" commits this sprint.  Either canonicalise one and
    have the others link, or add a `make docs-check` script that
    greps for FIXED markers in the long form and complains when the
    Quick-Reference still says open.

11. **Memory of recent decisions.**  DESIGN_DECISIONS.md exists for
    the closed-by-decision register but isn't yet referenced from
    PLANNING.md or PROBLEMS.md headers.  A one-line "Before
    proposing a feature, check DESIGN_DECISIONS.md" at the top of
    each would prevent the same five proposals returning every
    quarter.

12. **A `make ship` target.**  Today's release gate is
    `cargo fmt && cargo clippy --release --all-targets -- -D warnings
    && cargo test --release`.  The `--no-default-features` clippy
    run is easy to forget; the full suite gets skipped when only one
    test changed.  A single make target that runs all four (and
    refuses to push if any fails) prevents the stale-doc /
    forgotten-test scenarios that produced this sprint's hygiene
    commits.

---

## Recommended landing order

1. **B7** — unblocks 5 of the 13 P54 ignored tests.
2. **P54 step 4** — array/object arena materialisation.  Q2, Q3,
   Q4 all depend on this.
3. **Q2** — `kind` / `keys` / `fields` / `has_field`.  Cheap; makes
   free-form actually free-form.
4. **Q1** — structured `ParseError` + path + line:column + context
   snippet.  Parser side first; the helper functions then get
   reused by step 5 and Q3.
5. **P54 step 5** — `Type::parse(JsonValue)` with the field-type
   matrix + strict / permissive policy.  Reuses Q1 infrastructure
   for schema diagnostics.
6. **Q4** — `json_null` / `json_bool` / … / `json_object`
   constructors.  Bypasses B2-runtime by allocating in Rust.
7. **Q3** — `to_json` / `to_json_pretty` + `T.to_json()` codegen.
   Round-trip tests become possible.
8. **P54 step 6** — migrate `tests/scripts/57-json.loft` +
   `tests/docs/24-json.loft` off `Struct.parse(text)`; ship the
   rejection diagnostic.
9. **P54 steps 7-8** — unignore remaining P54 tests; doc sweep.
10. **C54.A** — integer i64 widening.  Schedule last in 0.9.0 so
    earlier bites are fixed on the existing layout before the
    schema bump.
11. **C54.C → B → E** — sub-tickets in order.

Tier 2 items run in parallel as session-of-the-week background
bites.  Tier 3 / 4 — at most one per release window.

---

## See also

- [PROBLEMS.md](PROBLEMS.md) — historical bug log (interpreter
  robustness, web services, graphics)
- [CAVEATS.md](CAVEATS.md) — verifiable edge cases with reproducers
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — language design
  asymmetries
- [DESIGN_DECISIONS.md](DESIGN_DECISIONS.md) — closed-by-decision
  register
- [PLANNING.md](PLANNING.md) — priority-ordered enhancement backlog
- [ROADMAP.md](ROADMAP.md) — items grouped by milestone
- [DEVELOPMENT.md](DEVELOPMENT.md) — branching, commit order, CI
