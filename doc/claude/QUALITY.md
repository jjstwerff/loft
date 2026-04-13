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
| B5 | Recursive struct-enum — `vector<Tree>` field allocates with `db_tp=u16::MAX` | Medium | Compiler — **further-narrowed 2026-04-13** (post-B7 fix): interpreter-only.  Inside `n_count`, an `OpDatabase(var=12, db_tp=0xFFFF)` fires.  Trace shows `var_t: Tree` parameter is fine; the failing op is for an internal slot allocated for the iteration — likely the for-loop's `__vector_N` work-ref or `kids` binding.  Native-codegen schema does NOT register `main_vector<Tree>` (it isn't in the structures list), so when `gen_set_first_vector_null` looks up `name_type("main_vector<Tree>")`, it returns `u16::MAX` — emitted into bytecode and causes the runtime panic.  **Fix path:** ensure `main_vector<Tree>` (and main_vector for any struct-enum element type) is registered during `fill_database` of the parent struct/enum that contains it, OR detect the missing registration in `gen_set_first_vector_null` and either auto-register or emit a parser-level diagnostic.  The non-recursive case `vector<JV>` works because the field is inside a struct (Box.items) where field-type resolution uses a different path that doesn't need `main_vector<JV>` |

Items that look open in the historical sections of PROBLEMS.md /
CAVEATS.md but are now closed: P22, P91, P135 / C58, P137, P139, C60,
INC#3, INC#29, P140 (test-harness reordering 2026-04-13), P141 (false
positive — `x#continue` already works), B3 (ref_return Enum arm
2026-04-13 — struct-enum return types now get hidden caller
pre-alloc args just like Reference/Vector), B2-runtime (2026-04-13
— 4-part fix: fill_all enum-field retrofit, parse_constant_value
v_block wrap, fields.rs Sig.Idle wrap, calc.rs sub-struct size=1),
B7 (2026-04-13 — `def_code` null-code path now sets `self.arguments`
and `stack.position` from def attributes for native fns + native
registry aliases `t_9JsonValue_<method>` → `n_<method>` impls).
See CHANGELOG.md.

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

**Fix design (original — stale; see revised note below).**
Unit variants in **mixed** enums (where some variants have fields
and some are unit) leave the payload buffer uninitialised when
constructed at runtime.  The variant tag byte is set correctly,
but the residual bytes beyond the tag carry whatever was on the
stack — match dispatch then reads garbage and either fails to
match or matches the wrong arm.

**Re-diagnosis 2026-04-13** (via `LOFT_LOG=full` on the same
`Sig { Off, Idle, On { level } }` reproducer): the observed
runtime symptom is **not** the predicted garbage-tag mismatch.
Instead, the test loops returning `value=16` thousands of times
until the harness's "Too many operations" guard fires at
`src/state/debug.rs:974`.  The match expression seems to
re-enter the function rather than exit it, which suggests a
codegen issue at the match-dispatch / return-slot layer, not
(only) the parse-time zero-fill.  Before attempting surgery,
capture a narrower trace with `LOFT_LOG=fn:run` and read
`parse_enum_field` → `parse_object` → match-arm return path
together.  Zero-filling the payload is likely necessary but
not sufficient.

**Partial fix landed 2026-04-13.**  Two root causes identified:

1. **Type-layout (LANDED):** `parse_enum_values` only added the
   "enum" discriminant attribute to struct variants (those with
   braces), leaving sibling unit variants with 0 attributes.
   `fill_database` then produced size-0 structures for them, and
   `Store::claim(size=0)` panicked "Incomplete record".  Fix in
   `src/typedef.rs::fill_all`: retroactively add the "enum" field
   to every unit variant whose parent's `returned` is a mixed
   `Type::Enum(_, true, _)`.  Off/Idle/On now all have the
   discriminant field in the native-schema emit.
2. **Bare-identifier construction (LANDED 2026-04-13):** extended
   `parse_constant_value` at `src/parser/objects.rs:481` to emit an
   inline `v_block` when the resolved variant's parent enum is
   mixed.  The block allocates a work-ref DbRef, calls
   `object_init` (which writes the discriminant via the "enum"
   field's default value), and returns the work-ref.  Work-ref is
   marked `skip_free` so only the receiving slot (var_s) frees the
   store at scope exit.  Native-emit verified: `let var_s: DbRef =
   { OpDatabase(__ref_1, 61); set_byte(0)=2; __ref_1 };` with
   single `OpFreeRef(var_s)`.

3. **Interpreter codegen (NOT landed):** `state::execute` still
   panics `Incomplete record` on the same reproducer, meaning the
   bytecode generation in `src/state/codegen.rs` doesn't observe
   the new `v_block` form the same way native-emit does.  Layer 1+2
   pass at the IR + native-Rust output level; the interpreter's
   bytecode emitter needs paired handling for
   `Type::Enum(_, true, _)` destination slots receiving a
   v_block containing an `OpDatabase` + field-init sequence.
   Follow `gen_set_first_ref_*` sites for the struct-Reference
   path and mirror for struct-enums.  Est. 1 session.

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

**Re-diagnosis 2026-04-13** (via `LOFT_LOG=crash_tail:30` on
`p54_b3_float_via_intermediate`).  The observed failure is not a
deep-copy / free-collision; it is `n_mk` **calling itself
infinitely** from its own tail position.  The tail expression `n`
(a local of struct-enum type) compiles to an `OpCall(fn=n_mk, …)`
each time — a fresh store is allocated (`ConvRefFromNull` →
`Database`) and the body re-executes before any return.  The heap
grows by one store per iteration until `free(): invalid next
size` aborts.

This sharpens the original 4-layer design: layer 1
(caller-side hidden-slot pre-alloc when the callee returns
`Type::Enum(_, true, _)`) is the site that currently mis-routes
the tail `n` load as a recursive call.  Without a reserved
return-slot the codegen falls back to the "call expression" path
for the tail local, and the return slot never materialises.
Landing layer 1 first, rerunning the trace, and only then adding
layers 2-4 is now the recommended order (instead of landing all
four together as the original design required).

**Fix design (original 4-layer, still applicable).**
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

**B5 — Recursive struct-enum runtime crash** (re-diagnosed further
2026-04-13).  The failure is now known to be a **test-harness divergence**,
not a general interpreter bug.  The same loft source:

```loft
pub enum Tree { Leaf { v: integer }, Node { kids: vector<Tree> } }
fn count(t: const Tree) -> integer {
    match t {
        Leaf { v } => v,
        Node { kids } => { c = 0; for k in kids { c += count(k); }; c }
    }
}
fn run() -> integer {
    root = Node { kids: [Leaf { v: 3 }, Leaf { v: 4 }] };
    count(root)
}
fn main() { println("{run()}"); }
```

* `cargo run --release --bin loft -- rec.loft` → prints `7` ✓
* `cargo test --release --test issues p54_b5 -- --ignored` → panics
  `assertion failed: size >= 1: "Incomplete record"` at
  `src/store.rs:221`, under an `OpDatabase` op whose `db_tp` is `0xFFFF`.

Further reductions pin down the difference:

* `build() -> Tree` alone (construct + match, no recursion) — both paths ok.
* `count(t)` with `return match ... v, x`, called non-recursively — both ok.
* `count(t)` recursive on `[Leaf, Leaf]` siblings — standalone ok, harness fails.

So the harness-only divergence lives in `tests/testing.rs`.  Candidates:
`cached_default()` (shared state across tests — may retain stores
populated by an earlier test); the parse path via `parse_str` vs
`main.rs`'s primary entrypoint; the always-called `generate_code`
native-codegen side effect on line 219.  Workaround: arena indirection
(P54 step 0).  Test: `p54_b5_recursive_struct_enum` (`#[ignore]`).

**Fix approach (revised 2026-04-13).**  Skip the typedef surgery the
earlier design targeted — the live-binary path shows `fill_database` /
`known_type` propagation is correct in the general case.  Instead:

1. Re-run the harness with `LOFT_LOG=variables` and `LOFT_LOG=fn:count`
   on the failing case and diff against the passing live-binary trace to
   locate the exact op where `db_tp` diverges.
2. If the divergence is in `cached_default()`: give each test a fresh
   `Stores` rather than a clone of the cached one for struct-enum
   scenarios.
3. If it's the always-on `generate_code` call at `tests/testing.rs:219`:
   hoist it behind the same `log_active` gate the other instrumentation
   uses, so it doesn't mutate state before `state.execute` runs.

**Estimated scope:** one session once the divergence op is located.

**Fix design (revised 2026-04-13 after runtime trace).**  The original
"memoise `fill_database`" surgery appears to be in-tree already
(compile now succeeds on the same `Tree` reproducer that tripped
the 500-depth guard in earlier writeups).  What remains is the
**known_type handshake** for recursively-defined cells.
`fill_database` has branches (lines 249-253 for `Type::Vector`,
similar for Hash/Sorted/Index/Spacial) that read
`data.def(c_nr).known_type` and, when it's `u16::MAX`, recurse via
`fill_database(data, database, c_nr)`.  For a self-referential
content type (`vector<Tree>` where the cell type is Tree itself,
still in progress), the recursion is skipped by the memoisation
but `c_tp` stays at `u16::MAX` — and that MAX propagates into the
enclosing def's known_type entry, then into runtime `OpDatabase`
ops, then into `Store::claim(size=0)` → panic.

**Fix:** when `fill_database` detects re-entry for a type already
in progress, **don't return the unresolved `u16::MAX`** — either
(a) return a sentinel struct type sized to fit `DbRef` (12 bytes)
for the recursive cell, since recursive content is always
heap-allocated through a DbRef anyway, or (b) add a second pass
after all defs' top-level structures are allocated, which walks
the vector/hash/etc. cells that were deferred and fills them in
then.  Option (a) is the smaller change.

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

**Unification finding 2026-04-13.**  B7's signature — `~500 iterations of
Return(ret=0, value=16, discard=0) at PC=0` followed by legitimate code
resuming, followed by store-leak warning + double-free — **matches B2-runtime
and B3 trace-for-trace**.  All three fire `OpReturn` / `OpCall` in a loop at a
function boundary involving a struct-enum value.  Specifically:

* B2-runtime: `s = Idle; match s { ... }` — OpReturn loops after the match.
* B3: `fn mk() -> JV { n = A{..}; n }` — OpCall loops (function calls itself).
* B7: `len(v_b7m)` where `v_b7m: JsonValue` — OpReturn loops after len.

The common thread: the **caller's reserved return slot** is wrong-sized or
wrong-addressed for a `Type::Enum(_, true, _)` value.  OpReturn pops `value=16`
bytes (the DbRef size) but the stack pointer advances because the caller
reserved the slot incorrectly; eventually the stack unwinds to a non-zero PC
and normal code resumes.  A **single fix** to the return-slot reservation /
OpReturn accounting for struct-enums likely closes all three items together.

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

4. **~~`cargo clippy --no-default-features --all-targets -- -D warnings`
   cleanup.~~**  Landed 2026-04-13.  Full `--no-default-features`
   build now goes clean through clippy on both lib and bin targets
   and both feature combinations still compile identically.  Fix set:
   - **`src/parallel.rs`** — 12 original lints: 7× `needless_pass_by_value`
     and 2× `not_unsafe_ptr_arg_deref` suppressed via
     `#[cfg_attr(not(feature = "threading"), allow(…))]` (the value is
     consumed by `Arc::new(program)` in the threading branch, borrowed
     in the non-threading branch; making the public fn `unsafe` would
     cascade across every `par(...)` site).  3× `needless_range_loop`
     in the non-threading fallbacks refactored to
     `for (row_idx, r) in results.iter_mut().enumerate()`.
   - **`src/parallel.rs`** — cascaded `dead_code` on 5
     `run_parallel_*` fns + `WorkerPool::new`: same cfg-gated allow,
     the binary-crate view compiled by `main.rs` sees no callers
     under `--no-default-features`.
   - **`src/main.rs`** — `extract_toml_version`, `chrono_date`,
     `days_to_ymd` moved under `#[cfg(feature = "registry")]`; the
     `registry_sync()` tail-body (formerly reached only when
     `registry` is enabled) wrapped in `#[cfg(feature = "registry")]`
     to resolve the `unreachable_code` warning that fired after the
     cfg'd-out branch's unconditional `exit(1)`.
   - **`tests/data_structures.rs`** — the lone `index_deletions` test
     that uses `rand_pcg::Pcg64Mcg` gated behind
     `#[cfg(feature = "random")]`; its imports the same.
   - **CI** — `Makefile` `ci:` target now invokes
     `cargo clippy --no-default-features --all-targets -- -D warnings`
     alongside the default-features gate, so the ratchet stays
     green on every push.
   - **Regression guard** —
     `tests/doc_hygiene.rs::ci_target_runs_no_default_features_clippy`
     reads the Makefile and fails if the gate is ever removed.

5. **Migrate `Struct.parse(text)` → `json_parse(text) → match`** in
   `tests/scripts/57-json.loft` and `tests/docs/24-json.loft` once
   P54 step 5 lands.  Unblocks step 6 (the rejection diagnostic)
   and turns the tests into examples of the modern API.

6a. **~~Drop `code!()`'s duplicate-test emission.~~**  Landed
    (investigation) 2026-04-13 — turned out to be a false positive: the
    `duplicate_macro_attributes` warning and "same test name printed
    twice" output both traced to a single orphan `#[test]`
    attribute in `tests/issues.rs` left over from a test-block
    move.  The `code!()` macro is clean.  Removed the orphan; added
    `tests/doc_hygiene.rs::no_orphan_test_attributes_in_tests_issues_rs`
    so the next orphan is caught at test time, not via a
    misattributed warning.  No further action.

6b. **~~Drift guard for `#[ignore]`'d tests.~~**  Landed 2026-04-13:
    `tests/doc_hygiene.rs::ignored_tests_baseline_is_current` loads
    `tests/ignored_tests.baseline` (name + reason per ignored test,
    20 rows today) and fails with a +/- diff when the set drifts.
    Regenerator at `tests/dump_ignored_tests.py`.  Catches
    un-ignored-without-baseline-update, silently-added new
    `#[ignore]`, and reason-string edits.  Does *not* yet run the
    ignored tests themselves and diff pass/fail/panic-message —
    that heavier nightly `--ignored` diff is the remaining gap.

6c. **~~Surface method-vs-free suggestions in diagnostics (both
    directions).~~**  Landed 2026-04-13 in `src/parser/fields.rs` and
    `src/parser/mod.rs`:
    - **method→free** (original): when field access fails and a free
      function `n_<field>` exists whose first parameter is compatible
      with the receiver type, the diagnostic now reads
      `"Unknown field vector.sum_of — did you mean the free function
      `sum_of(…)` ? (stdlib declared `sum_of` as free-only; see
      LOFT.md § Methods and function calls)"`.  Tests:
      `inc08_sum_of_is_free_function_only` locks the hint wording;
      `quality_6c_unknown_field_without_free_fn_has_no_hint` locks
      specificity (a genuinely-misspelled field still gets the plain
      message).
    - **free→method** (follow-on, landed same day): when a free call
      `name(…)` fails and a method `t_<LEN><Type>_<name>` exists on
      some other type (typically the user passed a wrong-type
      receiver to a `self:` method via free syntax), the diagnostic
      now reads `"Unknown function starts_with — did you mean the
      method `x.starts_with(…)` on text? (stdlib declared
      `starts_with` as a method; see LOFT.md § Methods and function
      calls)"`.  Methods declared on multiple receivers (e.g.
      `is_numeric` on both `text` and `character`) are enumerated
      with `/`.  Site: `src/parser/mod.rs::call` uses
      `find_method_receivers` to scan definitions for the
      `t_<LEN><Type>_<name>` pattern.  Tests:
      `quality_6c_free_call_on_wrong_type_suggests_method`,
      `quality_6c_free_call_lists_all_method_receivers`,
      `quality_6c_free_call_unknown_fn_has_no_method_hint` (negative
      — a genuinely-unknown name still prints the plain message).

6d. **~~Better errors for keyed-collection construction.~~**  Landed
    2026-04-13 in `src/parser/fields.rs::index_type`: the
    `"Indexing a non vector"` diagnostic now spells out both the
    missing feature (no generic-constructor expression) and the
    idiom that works (struct-field declaration + vector-literal
    initialisation).  Tests: `quality_6d_keyed_collection_constructor_hint`
    locks the new wording on the `hash<Row[id]>()` reproducer;
    `tests/parse_errors.rs::index_non_indexable` updated to the
    new text on its `v = 5; v[1]` baseline.  Implementing the
    generic constructor itself is a separate, larger task — not
    this diagnostic fix.

6. **Document one inconsistency per session.**  Following the
   INC#3 / INC#12 / INC#26 / INC#29 pattern — write the gotcha into
   LOFT.md, lock the behaviour with 2-3 regression tests.  INC#2
   (vector-vs-keyed-collection API gap), INC#8 (method-vs-free-function
   stdlib choice), INC#18 (`x#break` labelled-break syntax), and INC#27
   (no `x#continue` counterpart — silent bare-continue) all landed
   2026-04-13.  No further INC doc-bite candidates remain; future
   sessions should draw from Tier 1 or Tier 3 backlog items.

### Tier 3 — structural, larger payoff

7. **~~Bytecode cache verification.~~**  Landed 2026-04-13 in
   `tests/bytecode_cache.rs`.  `.loftc` shipped in commit `4039490`;
   the hit / miss / invalidation cycle is now locked with four
   process-level tests that drive the real `loft --interpret` binary
   end-to-end:
   - `first_run_writes_loftc_with_magic_header` — fresh compile
     creates `.loftc` next to the source, beginning with the `"LFC1"`
     magic bytes.
   - `second_run_reuses_cache_bytes_unchanged` — two consecutive runs
     on the same source leave `.loftc` byte-identical (hit path).
   - `source_change_invalidates_and_rewrites_cache` — editing the
     source changes the SHA-256 key; `.loftc` is rewritten and the
     new stdout reflects the new source (not a stale cached image).
   - `missing_loftc_is_recreated` — deleting the cache file between
     runs forces regeneration on the next run.

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
    - **Landed 2026-04-13:** `tests/doc_hygiene.rs` now guards all
      four sources — INCONSISTENCIES.md (Status blocks ↔ Resolved
      table), PROBLEMS.md (Quick-Reference ↔ long-form
      `### ~~N~~ FIXED` headings), CAVEATS.md (long-form
      `### ~~CX~~ DONE` ↔ Verification-log table), and QUALITY.md
      itself (main open-issues table must contain no crossed-out
      rows; Tier-2 strikethrough items must carry a `Landed
      YYYY-MM-DD` marker in their body).  Caught five existing
      drifts on first runs: #135 (PROBLEMS Quick-Reference), P137,
      C58/P135, and C60 (CAVEATS Verification-log), plus 6a's
      missing landing marker (QUALITY self-guard) — all corrected
      in the same commits.  Item 10's scope is now closed; future
      drift gets caught in CI instead of sprint-hygiene commits.

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
