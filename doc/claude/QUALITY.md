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
| Q1 | `json_errors()` reports byte offset only — no path, no line:column, no context snippet | Medium | **Parser side shipped** — RFC 6901 path + line:column + context snippet now in `json_errors()`.  Schema-side reuse (P54 step 5) still pending |
| Q2 | No free-form object iteration / key listing / quick `kind(v)` peek | Medium | **Designed, not landed** — see § Q2 below |
| Q3 | No `to_json(v)` serialiser — reads but can't write or round-trip | Medium | **Designed, not landed** — see § Q3 below |
| Q4 | No way to construct `JsonValue` trees in loft code (fixtures, mocking, forwarding) | Medium | **Designed, not landed** — see § Q4 below |
| C54 | `integer` arithmetic on `i32::MIN` silently returns null | Medium | **Designed, not landed** — see § C54 below |
| B2-runtime | Unit-variant literal construction in struct-enum crashes | Medium | Compiler — **fix designed** (zero-fill payload at `src/parser/objects.rs::parse_enum_field`); 1 session |
| B3 | Struct-enum tail-expression return crashes | Medium | Compiler — **fix designed** (4-layer codegen surgery); 2-3 sessions |
| B5 | Recursive struct-enum trips codegen recursion guard | Medium | Compiler — **fix designed** (memoise `fill_database` in `src/typedef.rs`); 1 session |
| B7 | Native-returned struct-enum temporaries leak intermediate stores | Medium | Compiler — single-line fix tried 2026-04-13 and **did NOT close the bug**; revised estimate 2-3 sessions; design + investigation candidates in § B7 below |

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

**Status (2026-04-13).**  Parser side **shipped**.
`src/json.rs::parse` returns `Result<Parsed, ParseError>` carrying
`message`, `byte_offset`, and an RFC 6901 `path`.  Path-stack is
threaded through `parse_object` / `parse_array`.  `format_error`
builds the line:column + context snippet on demand.  `n_json_parse`
calls it; `json_errors()` returns the rich text.

```
err: parse error at line 1 col 9 (byte 8):
  path: /x
  expected digit after `.`
    1 │ {"x": 1.}
      │         ^
```

8 unit tests in `src/json::tests` (path for root / array index /
object field / nested / RFC 6901 escapes; line:col conversion;
format_error covering path / line / col / caret) plus 4
acceptance tests in `tests/issues.rs::q1_*` (path for object
field, path for array index, caret marker present, line+byte
markers present).

**Schema-side still pending**: `Type::parse(JsonValue)` failures
will reuse the same path + format_error infrastructure when P54
step 5 lands.  Recovering parser (continue past first error,
return list of failures) remains a follow-up with its own
trade-offs.

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

**Status (2026-04-13):** Concrete fix designs documented for all
four open compiler bugs (B2-runtime, B3, B5, B7) following an
explore-agent investigation.

The B7 single-line-fix prediction was tested and **did NOT close
the bug** — the type-match extension at `src/scopes.rs:1031` is
necessary but not sufficient; at least one other site in the
lifecycle machinery is also wrong.  Revised B7 estimate: **2-3
sessions** with `LOFT_LOG=full` instrumentation to pinpoint the
duplicate OpFreeRef emission.  Design + candidate sites listed in
§ B7 below.

The B5 / B2-runtime / B3 fix designs remain untested but
file:line targets are concrete.  Recommended landing order
restored to "B7 first, then B5, then …" because B7 still has the
largest blast radius even at the higher cost.

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

**Fix design (added 2026-04-13 from explore-agent investigation).**
Unit variants in **mixed** enums (where some variants have fields
and some are unit) leave the payload buffer uninitialised when
constructed at runtime.  The variant tag byte is set correctly,
but the residual bytes beyond the tag carry whatever was on the
stack — match dispatch then reads garbage and either fails to
match or matches the wrong arm.

**Site:** `src/parser/objects.rs::parse_enum_field` (lines 1286-1314)
constructs the variant struct via `parse_object(e_nr, &mut cd)`.
For unit variants (0 attribute fields), the underlying
`OpDatabase` allocates a record but no field-init writes follow,
so the payload bytes stay garbage.

**Fix:** in `parse_enum_field`, detect the unit-variant case
(`def.attributes.is_empty()` for the variant struct) and emit a
zero-fill of the payload region after the `OpDatabase` /
`OpSetEnum` calls but before returning the value.  The payload
region size is `size(parent_enum) - 1` (everything after the tag
byte).  Reuse the existing bulk zero-fill op in `src/fill.rs` (or
add a 5-line `op_zero_bytes` handler if no exact op exists).

**Files:** `src/parser/objects.rs:1286-1314`; possibly a new op in
`default/01_code.loft` and `src/fill.rs`.
**Estimated scope:** one session.

**Verification path:**
1. `cargo test --release --test issues p54_b2_runtime_*` —
   2 currently-`#[ignore]`'d tests flip to green
   (`p54_b2_runtime_unit_variant_construction`,
   `p54_b2_runtime_qualified_unit_variant_in_mixed_enum`).
2. Full suite green.
3. Smoke: `JsonValue.JNull { is_null: true }` constructed at
   runtime in user code matches correctly via
   `match v { JNull { is_null } => ... }`.

**Side-effect risk:** low.  The fix narrows behaviour
(garbage-payload → zero-payload), making previously-undefined
match results well-defined.  Programs that accidentally relied on
the garbage value were already broken.

**B3 — Struct-enum tail-expression return crashes.**  Five
investigation sessions narrowed the diagnosis: needs **at least
4 coordinated codegen layers** changed (caller-side hidden-slot
allocation, `scopes.rs:307-318` hoist, `OpCopyRecord` deep-copy paths,
OpReturn discard accounting).  Single or even 3-layer attempts mutate
the symptom but never close it.  Workaround: explicit `return n;`
instead of `n` at function tail.  Tests: `p54_b3_*` (`#[ignore]`).
Estimated 8-12 source-line ranges across 2 files when attempted as
one focused refactor.

**Fix design (added 2026-04-13 from explore-agent investigation).**
Four coordinated layers must change.  Concrete file:line targets:

| Layer | File | Line(s) | Change |
|---|---|---|---|
| 1. Caller pre-alloc | `src/state/codegen.rs::generate_call` | 1410-1420 (before OpCall emission) | When callee's return type is `Type::Enum(_, true, _)`, emit `OpDatabase` for a 12-byte return slot, mirroring the Reference path |
| 2. Hoist | `src/scopes.rs` | 311 | Extend the hoist-set match from `Type::Reference \| Type::Vector` to also include `Type::Enum(_, true, _)` |
| 3. Deep-copy | `src/state/codegen.rs` | 827, 954-960, 975-1022, 1080, 1101, 1112-1130 | Every `Type::Reference` arm in OpCopyRecord-related match sites grows an `\| Type::Enum(_, true, _)` sibling |
| 4. Type extract | `src/state/codegen.rs::known_type` | 1761-1763 | Match arm currently extracts `Type::Reference(c, _) → c`; extend to `Type::Reference(c, _) \| Type::Enum(c, true, _)` |

**Estimated scope:** 2-3 sessions.  Each layer is independent and
testable; if a session lands only layers 1-2, the symptom mutates
but doesn't close — five investigation sessions confirmed all four
must land together.

**Verification path:**
1. After all 4 layers land: `cargo test --release --test issues p54_b3_*`
   — 4 currently-`#[ignore]`'d tests flip to green
   (`p54_struct_enum_explicit_return_of_local` already passes via
   the `return n;` workaround; the implicit tail-expression form is
   what the fix covers).
2. Full suite green.
3. Manual smoke: write the original BITING_PLAN reproducer
   (`fn mk() -> JV { A { v: 42 } }`) and confirm no crash.

**Side-effect risk:** medium.  OpCopyRecord deep-copy paths are
load-bearing for vector/struct passing; extending each match arm
needs a matching test for the new Enum case to avoid regressing
the existing Reference path.

**Why B3 sits *after* B7 in the recommended order:** they're
independent codegen surgeries with no overlap, and B7 unblocks
5x more downstream work per line of code touched.  B3 closes an
ergonomics gap; the `return n;` workaround stays good for any
user who needs it.

**B5 — Recursive struct-enum infinite codegen loop.**  Declaring
`JArray { items: vector<JsonValue> }` trips the
`Recursion depth limit exceeded (500)` guard.  Workaround:
arena indirection (P54 step 0).  Test: `p54_b5_recursive_struct_enum`
(`#[ignore]`).

**Fix design (added 2026-04-13 from explore-agent investigation).**
The 500-limit guard at `src/state/mod.rs:197` is a downstream
runtime call-depth guard, **not** the cause.  The actual recursion
is in compile-time type layout: `src/typedef.rs::fill_database`
walks struct/enum field types to compute storage sizes, and a
field whose type is the parent enum (e.g.
`JArray { items: vector<JsonValue> }`) re-enters `fill_database`
for `JsonValue` 500+ times before the runtime guard catches the
symptom.

**Fix:** memoise visited type definitions in `fill_database`.
Pass an `&mut HashSet<u32>` of in-progress `d_nr`s; on re-entry
for an already-in-progress type, return the partially-computed
size (or a sentinel "self-referential, defer") and continue.
Reference-typed fields are already fixed-size (12 bytes for
`DbRef`) so the cycle terminates trivially once memoised.

**File:** `src/typedef.rs:209-311` (`fill_database`).
**Estimated scope:** one session.

**Verification path:**
1. With the fix applied, the recursive enum form
   ```loft
   pub enum JsonValue {
       JArray  { items: vector<JsonValue> },
       JObject { fields: vector<JsonField> },
       ...
   }
   pub struct JsonField { name: text, value: JsonValue }
   ```
   compiles instead of tripping the 500-depth guard.
2. `cargo test --release --test issues p54_b5_recursive_struct_enum`
   flips from `#[ignore]` to green.
3. Once B5 is fixed, **P54 step 4 simplifies** — the
   arena-indirection workaround (`items_id: integer` + a separate
   `vector<JsonValue>` arena) becomes optional rather than
   compelled by codegen.  The arena workaround can stay for
   performance reasons (one allocation vs. many) but the natural
   `vector<JsonValue>` form also works.

**Side-effect risk:** low.  Memoization in compile-time layout is
a standard transform; no runtime behaviour change.  Test exposure:
add a test compiling a self-referential enum with a non-recursive
base case (e.g. a simple Tree<T>) to lock the memoization works
correctly.

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
- **(new symptom — INC#9 caveat)** `fn f() -> text { c = txt[0]; "{c}" }`
  SIGSEGVs.  The text built via `n_format_text` on a character
  isn't tracked for free on the outer function's text-return
  path.  Regression guard: `b7_character_interpolation_return_crashes`
  (`#[ignore]`'d).

**Retraction** (2026-04-13): an earlier note claimed "a second
`json_parse` call in the same function corrupts memory."
Investigation while writing B7 regression tests showed that
multiple `json_parse` calls work fine when each result is
consumed via pattern matching — the corruption observed in
earlier smoke tests came from the subsequent `kind()` / `len()`
method calls, not from `json_parse` itself.  Guard for the
working multi-parse path: `b7_multiple_json_parse_via_match_works`.

**Blast radius**: the entire `(JsonValue) -> T` method surface is
gated on this fix, not just text returns.  This means **Q2**
(`kind`, `keys`, `fields`, `has_field`), **Q3** (`to_json`,
`to_json_pretty`), the planned step-4 implementations of
`field`/`item`/`len`, and parts of step 5 (`Type::parse(JsonValue)`)
all sit downstream.

**Fix design (added 2026-04-13 from explore-agent investigation).**
The bug is in `src/scopes.rs::inline_struct_return` at line **1031**:

```rust
if let Value::Call(fn_nr, _) = val {
    let def = data.def(*fn_nr);
    if def.name.starts_with("n_")
        && def.code != Value::Null
        && let Type::Reference(d_nr, _) = &def.returned   // ← only Reference
    {
        return Some(*d_nr);
    }
}
None
```

The Set path at `scopes.rs:447-449` and `needs_pre_init` at line
1043 already accept `Type::Enum(_, true, _)`.  Only this lifting
site was missed — so native calls returning struct-enum (e.g.
`json_parse(...) -> JsonValue`) bypass lifting, the JsonValue
store is embedded in the argument frame, and the callee's exit
frees the store before the caller's `OpFreeRef` would have fired.

**Single-line fix (proposed by the design):**

```rust
&& let Type::Reference(d_nr, _) | Type::Enum(d_nr, true, _) = &def.returned
```

**Update (2026-04-13, after attempted ship):** the single-line fix
was applied and the test `b7_method_on_jsonvalue_returning_integer_*`
*still crashed* with the same "stores not freed" + "double free or
corruption" pattern, in both the inline form
(`len(json_parse(...))`) and the assigned form
(`v = json_parse(...); len(v)`).  The fix was reverted.

The type-match was demonstrably incomplete (the Set path and
`needs_pre_init` already accept `Type::Enum(_, true, _)` —
`inline_struct_return` was the only outlier), but **necessary is
not sufficient**.  At least one other site in the lifecycle
machinery must also be wrong.  Candidates to investigate next:

1. `n_json_parse` may be allocating with the wrong initial
   ref-count — `stores.database()` returns a fresh store; if the
   initial ref-count is 1 but the caller's `OpFreeRef` is also
   wired to decrement, an unrelated path may also be issuing a
   free.  The original P54 design plan (Step 1) called this out
   as the B7 root cause: "allocate the arena store inside the
   caller's variable's store, not via `stores.database()`".
2. `get_free_vars` (lines 728-779) may be emitting OpFreeRef
   *twice* for variables that received a struct-enum from a
   native call — one path through the Set-from-call site at
   line 447-449 and a second through the variables-cleanup loop.
3. The interpreter codegen `state/codegen.rs:1043-1050`
   (referenced in the line 445 comment as the sibling skip-free
   logic) may need parallel treatment.

**Estimated scope (revised):** 2-3 sessions.  Not the one-line
fix the design predicted.  The first session's job: instrument
the run with `LOFT_LOG=full` for `b7_method_on_jsonvalue_returning_integer_crashes`
and identify which OpFreeRef site fires twice on the JsonValue
store.  Then fix the duplicate emission.

**Verification path:**
1. Run the currently-`#[ignore]`'d B7-family tests with `--ignored`
   and confirm they flip to passing:
   - `b7_method_on_jsonvalue_returning_integer_crashes`
   - `b7_character_interpolation_return_crashes`
   - `p54_extractor_as_text` and 3 sibling text-return tests
   - `p54_missing_chain_returns_jnull`
2. Then unignore them.
3. Confirm `b7_multiple_json_parse_via_match_works` (currently
   passing) stays green — guards the working multi-parse path.
4. Full suite green.

**Side-effect risk:** low.  Lifting was proven safe for
References in the P135 fix; extending to Enums preserves the
invariant (native function allocates and owns the store, lifted
temp takes ownership, OpFreeRef frees once at scope exit).  No
ref-count machinery involved.

**One fix turns 8 things green together**: 5 ignored P54 tests
(the text-return-through-fn family + chained-access) + 2
B7-prefixed guards + the INC#9 character-interpolation crash.
Highest-leverage compiler bite remaining and the bottleneck for
nearly every JSON deliverable on the roadmap.

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

**Updated 2026-04-13** — explore-agent investigation produced
concrete file:line targets for all four compiler bugs.  The B7
single-line-fix prediction was tested same day and did not close
the bug; revised estimate **2-3 sessions** (the type-match
extension is necessary but not sufficient — needs paired
investigation of the duplicate-OpFreeRef site).  B5 reclassified
from "needs arena workaround forever" to "one-session
memoization fix in `fill_database`".  Order rebuilt around these
findings.

1. **B7 — 2-3 session lifecycle fix** starting at
   `src/scopes.rs:1031` plus paired sites yet to be identified
   via `LOFT_LOG=full` instrumentation.  Still highest-leverage
   compiler bite — unblocks 5 ignored P54 tests + 2 B7-prefixed
   guards + the INC#9 character-interpolation crash + every
   `(JsonValue) -> T` method call.
2. **Q2** — `kind` / `keys` / `fields` / `has_field` natives
   become trivially shippable post-B7.  One session.
3. **B5 — memoise `fill_database`** in `src/typedef.rs`.  Removes
   the arena-indirection compulsion from P54 step 4 and unblocks
   future stdlib enums with recursive variants (Tree<T>,
   Result<T, E>, etc.).  One session.
4. **P54 step 4** — array/object materialisation.  Simpler
   post-B5 (natural `vector<JsonValue>` works); Q3 + Q4 unlock.
5. **Q1 schema-side reuse** — when P54 step 5 lands,
   `Type::parse(JsonValue)` reuses the already-shipped
   `format_error` infrastructure for per-field path diagnostics.
6. **P54 step 5** — `Type::parse(JsonValue)` codegen with the
   field-type matrix + strict / permissive policy.
7. **Q4** — `json_null` / `json_bool` / … / `json_object`
   constructors.  Bypasses B2-runtime by allocating in Rust;
   ships any time after step 4.
8. **Q3** — `to_json` / `to_json_pretty` + `T.to_json()` codegen.
   Round-trip tests become possible.
9. **B2-runtime — zero-fill unit-variant payload** in
   `src/parser/objects.rs::parse_enum_field`.  Quality-of-life for
   any user constructing struct-enum literals at runtime; not a
   P54 blocker (Q4 bypass already works).  One session.
10. **B3 — four-layer codegen surgery** for struct-enum tail
    returns.  2-3 sessions.  Closes the implicit-return ergonomics
    gap; the `return n;` workaround stays good for any user who
    needs it.  Lower priority than items 1-9.
11. **P54 step 6** — sweep stdlib/tests off `Struct.parse(text)`,
    ship rejection diagnostic.
12. **P54 steps 7-8** — unignore remaining P54 tests; doc sweep.
13. **C54.A → C → B → E** — integer i64 widening.  Schedule last
    in 0.9.0 so earlier bites are fixed on the existing layout
    before the schema bump.

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
