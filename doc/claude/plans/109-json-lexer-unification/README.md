<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 109 — Unify JSON tokenization on loft's own lexer

Tracker: [@PLN109](https://github.com/loft-lang/plans/issues/109) · `subject:loft` · `status:future`

## Status

Open — design ready, step 1b landed. Triggered by @PLN102 arc-E lib-audit **H5**
(JSON integers > 2⁵³ silently round through f64, corrupting even a known-type `integer`
field on deserialize). Rather than patch the duplicate scanner, retire it: drive JSON
tokenization from loft's own lexer, which already distinguishes int from float. **H5 is
fixed as a consequence.**

**loft2 branch state (2026-07-17):** on `tuxedo_work2`. **Phase 0 + Phase 1 DONE** (golden
corpus; JSON-mode lexer escapes/exponents; H5 exact-`Long` lex verified). **Phase 2 is
decomposed into 2a–2e** with the byte-offset impedance resolved by **Option 1** (§ Phase-2
finding). Phases 2–5 open — see § Sub-arcs + § Execution.

**Scope resolved (owner, 2026-07-17):** UNIFORM integer semantics — typed and generic
behave identically; the only change is that an integer-shaped number (no `.`, no exponent,
fits i64) is assumed to be an i64, realized once via `JsonValue::JInteger` so both paths
read it exactly. See § Phase-0 finding → **Decision**.

## Goal

Delete the hand-rolled Rust JSON scanner in `src/json.rs` and re-drive `crate::json`'s
parse on **loft's own lexer** (`src/lexer.rs`, via a JSON mode), keeping `Parsed` as the
stable output interface so consumers barely change. One lexer to maintain; integer JSON
values preserved to i64 on the typed path.

## Effort + design

- **Effort:** M (the lexer additions are small — validated 2026-07-17; the bulk is the
  parser rewrite + updating 22 `Parsed::Number` consumer sites, both mechanical and
  differential-tested).
- **Design:** ✓ (phasing + gap analysis below; load-bearing assumptions probed — § Validation).
- **Last touched:** 2026-07-17.

## The hard constraint — NO HYBRID (owner)

No runtime flag, no persistent dual-path, no default-to-old fallback. **A lingering
hybrid hides problems** — you cannot tell which parser ran, and a divergence goes
unnoticed. The old-vs-new differential check is a **throwaway migration scaffold** (a
test harness), deleted in Phase 4; the hand-rolled scanner is removed **completely**.
The end state is one lexer and one parse path. "Small safe steps" means *build-prove-swap-
delete in sequence*, never *ship both behind a switch*.

## Why now (root cause)

The generic JSON parser was built early (agent work); the typed and generic paths were
then **quickly merged** — `Struct.parse(text)` became `struct_from_jsonvalue(json_parse(text))`,
routing known-schema parsing through the schema-blind f64 tree. So the number's type is
lost (`crate::json::Parsed::Number(f64)`) *before* the field type is consulted. loft's
lexer already lexes `Integer(u32,bool)` / `Long(u64)` / `Float(f64)` / `Single(f32)` —
the distinction we need — and the JSON parser already mirrors loft's recursive-descent
parser structure, so the consolidation is natural, not a rewrite. Serialize is already
exact (verified `{"id":9007199254740993}`); this is **deserialize-only**.

## Scope decision (owner, 2026-07-17) — loft is not a JSON validator; reuse, don't re-implement

Two decisions that shrink this to "not much":

- **Reuse loft's number lexer with all its allowances.** loft's number reader (int/float,
  exponents, `_` separators, `0x`/`0b`/`0o`) is a *feature* for loft's **own serialized
  format** (registry / snapshots / RPC / IR schema — number-and-structure, no JSON-string
  concerns), and for external JSON, accepting a *superset* is fine — **loft parses input,
  it does not validate JSON conformance.** So there is **no separate strict-JSON number
  reader**; a stricter mode is possible but not currently needed.
- **The goal is to ACCEPT valid JSON, not to REJECT non-JSON.** The only number gap that
  matters is a valid-JSON form loft would *reject*: loft's exponent handles `e`/`e-` but
  not uppercase `E` or `e+`/`E+` (verified: `1.5e3` → `1500` works). Closing that is a
  ~2-line completeness addition to loft's number lexer that also improves loft.

## Gap analysis — loft's lexer vs JSON (validated 2026-07-17)

The lexer *infrastructure* is reused (the `Lexer` char stream + `Position` + `Diagnostics`
+ the int/float `LexItem` tokens); only the JSON-specific reading differs, and less than
first thought:

| Concern | loft lexer today | Design (small) |
|---|---|---|
| **Numbers** | int/float/`Single`, exponent `e`/`e-`, `_`/hex/oct allowances | **REUSE as-is** (allowances are wanted). Add uppercase-`E`/`e+` for JSON-accept. Classify integer-shaped, i64-fitting → i64, else f64. `-` sign: loft lexes it separately, the JSON parser combines it. **No i128** (> i64 → f64, documented ceiling). |
| **Strings** | `string()` — interpolation **already gated on `interpolate_strings`** (off = `{`/`}` literal, the config path); `escape_seq` does `\"\\\t\r\n\0`, `\xNN`, `\u{NNNN}` | **REUSE `string()` with `interpolate_strings=false`.** Add JSON's `\uXXXX` (4 hex, no braces, **+ surrogate pairs**) and `\/` to the escape path. This is the one genuine addition. |
| **Structure / literals** | `{ } [ ] : ,` are tokens; `true`/`false`/`null` are identifiers | **REUSE as-is** — no lexer change; the JSON parser dispatches on them. `Dialect::Lenient`'s bare-identifier keys/values map naturally onto loft's identifier lexing. |

## The interface that does NOT change — `Parsed`

`Parsed` (Null/Bool/Number/Str/Ident/Array/Object/Constructor) is the output type every
consumer reads. Keep it; add an integer case (`Parsed::Int(i64)`). Only ~10 sites match
`Parsed::Number` (`ir_schema`, `rpc`, `registry_index`, `snapshot`, `registry_advisories`,
`native`, `database/structures`) — each gains an `Int` arm (read i64 directly; in a float
context, `Int`→f64). Consumers of the JSON *stdlib* (`native.rs` extractors + the typed
`populate_struct`) are the ones that gain exact integers. This keeps the blast radius at
the number-reading sites, not the whole registry/rpc/snapshot surface.

## Validation (2026-07-17) — the load-bearing assumptions, probed

The claims that would break "small safe steps" if false, checked against the code before
committing to the phasing:

- **VERIFIED — `Parsed` is the stable interface; blast radius is the number sites.** 22
  `Parsed::Number` matches across 7 files (`rpc`, `ir_schema`, `registry_index`,
  `registry_advisories`, `native`, `database/snapshot`, `database/structures`) — all read
  the `Parsed` *type*, none the scanner. Keeping `Parsed` (adding `Int`) confines Phase 3.
- **VERIFIED — the int/float distinction already exists** (`LexItem::Integer`/`Long`/
  `Float`/`Single`); H5 preservation falls out.
- **VERIFIED — loft's number lexer already handles exponents** (`1.5e3` → `1500`); only
  uppercase-`E`/`e+` are missing (Phase 1b).
- **VERIFIED — the string reader already supports no-interpolation** (`interpolate_strings`
  flag, the config path), so JSON strings reuse it; the sole genuine addition is `\uXXXX`
  (+surrogates) / `\/`.
- **VERIFIED — clean cutover point:** `parse()` / `parse_with(Dialect)` are the single
  entry points, so swapping their internals is a replacement, not a flag.
- **VERIFIED — serialize is exact** (`{"id":9007199254740993}`); deserialize-only.
- **REFINED — "use loft's lexer" = reuse the lexer *infrastructure*** (char stream,
  `Position`, `Diagnostics`, int/float `LexItem`s), **not** loft's loft-grammar scanners
  verbatim (its `number()` carries `_`/hex/field-dot logic; `string()` does interpolation)
  — but both are *reused with small tweaks*, not rewritten. So Phase 1 is small, and the
  bulk of the work is the (mechanical) parser rewrite + the 22 consumer arms.
- **PRESERVE — API beyond `parse`:** `parse_with(Dialect::{Strict,Lenient})`,
  `format_error`/`line_col_of`, `ParseError`, `Parsed::Ident`.

## Sub-arcs — the migration steps

| Step | Item | Status |
|---|---|---|
| 0 | **Characterize** — ✅ **DONE** — `tests/json_corpus.rs` + `tests/json_corpus.golden`: a bless-able `Debug`-string golden over a liberal corpus (66 entries: every number/exponent form incl. H5, all string escapes incl. `\uXXXX`+surrogate-pair+astral+`\/`, structure/whitespace, literals, error byte-offsets, `Dialect::Lenient` bare-key/ident/constructor, + the real registry-index sample & an RPC line). Throwaway (deleted Phase 4); `harness_is_not_vacuous` proves it can fail. Pins the current behavior the rewrite must preserve — incl. the H5 cells (`9007199254740993` → `Number(…992.0)`) that flip to `Int` in Phase 2. Noted: the current byte-scanner ALREADY decodes surrogate pairs + `\/`, so Phase 1a is about the new lexer path reproducing them (golden pins the target). | **Done** |
| 1 | **JSON reading in loft's lexer (small — validated)** — ✅ **DONE.** (a) `LexConfig::json()` + a `json_strings` flag gate JSON escapes in `string()`/`escape_seq`: `\/` and `\uXXXX` (four hex, no braces) with surrogate-pair combining — a lenient superset over loft's own escapes, OFF for all normal loft/config lexing (verified `loft_string_escapes_unchanged` + full lexer module green, so loft source semantics are byte-identical). Tests `json_string_escapes` (ASCII/BMP/surrogate/astral/`\/`/mixed). (b) ✅ uppercase-`E`/`e+` exponent completeness (golden `number_exponent_accepts_uppercase_e_and_plus_sign`). (c) integer-shaped→i64 is a Phase-2 *mapping* (loft's lexer already emits `Integer`/`Long` vs `Float`); the load-bearing H5 claim is **verified** by `json_number_classification` — `9007199254740993` lexes to exact `Long(u64)`, NOT rounded through f64. Structure / literals / whitespace / `Dialect::Lenient` identifiers reuse loft's lexing as-is. | **Done** |
| 2 | ✅ **DONE** — reimplemented `parse()`/`parse_with()` on the JSON-mode lexer (`parse_lexer` + `JParser` in `src/json.rs`), **byte-identical** to the retired scanner across the full Phase-0 golden (66 entries) AND every consumer suite (rpc/registry/ir_schema/plan25_json/data_structures). Integers stay `Number(f64)` (H5 rounding included); the `Parsed::Int` flip is Phase 3. Old scanner **deleted** (Phase-4 scanner-retirement folded in): net ~−330 lines in `json.rs`. Commits `4f7bc6c2` (lexer lone-surrogate fix), `0bb45588` (`-` token + findings), `b732a409` (parser + swap + delete). | **Done** |
| 2a | **Byte-offset mapper + value/literal skeleton.** A `ByteMapper` (line-starts table + UTF-8-width walk) maps the lexer's `Position{line, char-col}` back to the absolute `byte_offset` the old scanner reported. Drive `LexConfig::json()` via `peek()`/`cont()`; parse `null`/`true`/`false` (identifiers) + trailing-byte + root whitespace/BOM. Golden-gated. | Open |
| 2b | **Strings** → `Parsed::Str` from `CString` (JSON escapes already gated in Phase 1a). | Open |
| 2c | **Numbers** → `Parsed::Number(f64)` from `Integer`/`Long`/`Float`/`Single` (convert to f64 — byte-identical to the old scanner, H5 rounding included); combine the leading `-` token. **The `Int` flip is Phase 3, not here.** | Open |
| 2d | **Array / object / Lenient / Constructor** — nesting, `Object` key `byte_offset`s (via `ByteMapper`), bare-identifier keys + `Tag{…}` constructors, path stack for JSON-pointer diagnostics. | Open |
| 2e | **Error parity** — reproduce the old scanner's messages + `byte_offset`s + `path` for every error cell in the golden; `json_errors()` unchanged. Then swap `parse`/`parse_with`; golden unchanged. | Open |
| 3 | ✅ **DONE** — uniform integer semantics. `Parsed::Int(i64)` + `Parsed::as_i64`/`as_f64`; integer-shaped lexemes (no `.`/exp, i64-fitting) → `Int`; `JsonValue::JInteger{value:integer}` (last variant) + `JV_DISCR_INT=7`; `materialise_primitive_into` / `json_parse_into_stores` / `dbref_to_parsed` / `json_to_text_at` / `format.rs::write_jsonvalue` / `n_as_long`/`n_as_number`/`n_kind` + the native `t_9JsonValue_*` mirror + `unwrap_long`/`unwrap_int`/`unwrap_float` / `push_kind_mismatch` all handle `JInteger`; the ~18 Rust `Parsed::Number` consumer sites route through `as_i64`/`as_f64`; loft JNumber consumers (194/198 + issues classify) updated. **H5 fixed on BOTH backends, typed AND generic** (`{"id":9007199254740993}` → exact). Golden re-blessed (int flips only; overflow >i64 stays `Number`). | **Done** |
| 4 | ✅ **DONE (folded into Phase 2)** — the hand-rolled scanner is deleted; `crate::json` is a thin layer over loft's lexer. **No hybrid remains.** | **Done** |
| 5 | **Verify + freeze** — full suite both backends; corpus → regression suite; H5 typed-integer golden added; STDLIB.md JSON number semantics (int→i64, float→f64, > i64 ceiling) + lib-audit H5 → DONE. | Open |

## Execution — safe small steps (loft2 baseline, verified 2026-07-17)

The ../loft design's load-bearing claims were re-probed against **loft2's own tree** (the
two checkouts have diverged — loft2 lacks the sibling's @PLN102 H7–H9 / @PLN110 work), and
they all hold here:

- **Blast radius = exactly 22 consumer `Parsed::Number` sites across 7 files** —
  `rpc.rs`(1), `ir_schema.rs`(4), `native.rs`(3), `registry_index.rs`(2),
  `registry_advisories.rs`(2), `database/snapshot.rs`(2), `database/structures.rs`(8).
  (`src/json.rs` itself holds 12 more: the scanner + its own tests.) The design's "22 / 7
  files" is exact on loft2.
- **`Parsed` matches the design** (Null/Bool/Number(f64)/Str/Ident/Array/Object/Constructor);
  **no `Parsed::Int` yet** → the `Int` addition is clean.
- **`src/json.rs` uses zero `crate::lexer`** — a fully separate hand-rolled byte-scanner
  (`bytes: &[u8]`, index-based), confirming there is no existing coupling to unwind.
- **The Phase-4 delete targets** are the six byte-scanner fns in `src/json.rs`:
  `parse_value` (l.229), `parse_string` (l.342), `parse_number` (l.428), `parse_array`
  (l.475), `parse_object` (l.517), `parse_object_key` (l.563) — 1097 lines total.
- **`LexItem::Integer/Long/Float/Single`** all present; **`interpolate_strings`** defaults
  `false` on the config path (`LexConfig`); **`parse()` / `parse_with(Dialect)`** are the
  single entry points (l.103 / l.114). Step 1b's exponent completeness is landed here.

**Phase 0 execution (next):** build the golden corpus as a *throwaway differential
scaffold* (`#[cfg(test)]`, deleted in Phase 4) — capture the current `Parsed` tree for
every consumer's real inputs (the JSON stdlib tests, a registry index sample, an RPC
message, an IR-schema doc, a DB snapshot, an advisories doc), plus the escape/exponent/
lenient-dialect edge cases the § Falsification points name. The corpus is the executable
spec for Phase 2's "byte-identical `Parsed`, except the intended int-preservation."

### Phase-0 finding (2026-07-17) — the H5 chokepoint is DOUBLE; Phase 3 is wider than "add an Int arm"

Empirically characterized on loft2 (both backends), `{"id": 9007199254740993}` (2⁵³+1):

```
typed   Rec.parse(raw).id   = 9007199254740992   (H5: rounded)
generic json_parse().as_long() = 9007199254740992   (H5: rounded)
expected                     = 9007199254740993
```

Both loft paths lose the integer at a **float chokepoint**, and there are **two** of
them, because the loft-side `JsonValue` is itself f64-backed (`JV_DISCR_NUMBER` stores a
`float`; `native.rs:2839` `set_float`, `native.rs:2753` `dbref_to_parsed` → `Parsed::Number(f64)`):

1. **Typed** `Type.parse(text)` lowers (parser intercept `parse_type_parse`,
   `src/parser/fields.rs`) to `struct_from_jsonvalue(json_parse(text), kt)` — so it routes
   `text → json_parse → JsonValue(float, INT LOST) → dbref_to_parsed → Parsed(float) →
   populate_struct`. The int is gone at the **first** step, before `populate_struct` runs.
2. **Generic** `json_parse(text).as_long()` reads `JsonValue::JNumber{value: float}` directly.

**Consequence for the design.** Adding `Parsed::Int(i64)` (Phase 2) + an `Int` arm to
`populate_struct` (Phase 3) is **necessary but not sufficient** for the typed path: as long
as `Type.parse(text)` sources its data through the float `JsonValue`, the int is lost before
any `Parsed::Int` exists. Closing H5 for the typed path therefore **also** requires
**re-routing `Type.parse(text)`** to feed `populate_struct` a `Parsed` produced *directly*
by `crate::json::parse(text)` (with `Int`), bypassing the `json_parse → JsonValue`
materialisation. That is real Phase-3 work the "22 consumer arms" framing understates —
the arms are read-sites of an *already-built* `Parsed`, but the typed path must first be
made to *carry* a `Parsed::Int` to `populate_struct` at all.

**Decision (owner, 2026-07-17) — UNIFORM integer semantics, no path divergence.** Typed
and generic MUST behave identically. The *only* semantic change is the type a bare number
is assumed to have: **a JSON number with no fractional part is an i64** (integer-shaped →
`integer`), a number with a `.` (or that overflows i64) is a `float`. This is realized
once, in the shared representation, so both `Type.parse(text).field` and
`json_parse(text).as_long()` read the exact integer — there is deliberately **no** typed-
only path and no second walker (option A is rejected).

**Realization on loft2 — one representation, `JsonValue::JInteger{value: integer}`.**
Because loft2's typed path and generic path *share* the store-backed `JsonValue` walker
(`populate_struct_from_jsonvalue` reads the store `JsonValue`, not a Rust `Parsed`), giving
the number an integer representation in the store `JsonValue` fixes both at once:

- **loft type** — add `JInteger { value: integer }` to the `JsonValue` enum (`06_json.loft`)
  beside `JNumber { value: float }`. A store discriminant `JV_DISCR_INT`.
- **Rust `Parsed`** — `crate::json::parse` produces `Parsed::Int(i64)` for integer-shaped
  numbers (falls out of loft's lexer, which already lexes `Integer`/`Long` vs `Float`);
  `materialise_primitive_into` gets an `Int` arm → `JV_DISCR_INT`; `dbref_to_parsed` reads
  `JV_DISCR_INT` → `Parsed::Int`.
- **extractors** (`06_json.loft` + `native.rs`) — `as_long`/`as_integer` on a `JInteger`
  reads the i64 exactly; `as_number`/`as_float` on a `JInteger` converts i64→float;
  `kind()` on a `JInteger` returns `"JInteger"`.
- **typed populate** — for an `integer`/`long` field, `unwrap_long`/`unwrap_int` accept a
  `JV_DISCR_INT` source and read the i64 exactly (today they only accept `JV_DISCR_NUMBER`
  and `get_float`); for a `float` field fed a `JInteger`, convert i64→float.

**Blast radius (loft2, verified) — small and compile-loud.** loft-side JNumber consumers
are exactly 3 files (`06_json.loft`; `tests/scripts/194-text-producer-dest.loft` and
`198-null-text-format.loft`, which assert `kind()=="JNumber"` / `"[42|JNumber]"` on integer
`42` and flip to `JInteger`); Rust-side, `codegen_runtime.rs`/`database/format.rs`/
`native.rs`. Adding a `JInteger` enum case makes loft's matchers flag every non-exhaustive
`match jv { … }` at **compile time** — the design-protocol "make omission loud" property
(the N-site silence factor drops to ~zero).

**Behaviour change to acknowledge (intended).** `json_parse("42").kind()` goes from
`"JNumber"` to `"JInteger"`, and a bare integer's `JsonValue` variant changes — this is the
deliberate consequence of "assume i64", not a regression. Consumers wanting the old float
reading write `42.0` or call `as_number()`. **H5 closes fully** (both the typed field and
the generic `as_long`), exactly as the lib-audit framed it.

**One edge pinned:** *integer-shaped* = loft's `LexItem::Integer`/`Long` = no `.` **and no
exponent** — so `1e3` is a `float` (1000.0), matching loft's own number lexer and
mainstream JSON libraries (Python `json.loads("1e3")` → `1000.0`). The user's rule ("no
`.123` → i64") holds for the common case; exponent-bearing numbers stay float by this
loft-lexer-consistent refinement.

### Phase-2 finding (2026-07-17) — the byte-offset impedance; Option 1 (keep byte offsets)

Phase 2 is NOT the "mechanical rewrite" the row first implied — two impedances between the
byte-scanner and loft's lexer must be handled, and they are what decomposes Phase 2 into
2a–2e:

1. **Position model mismatch.** The old scanner reports **absolute byte offsets** —
   `ParseError.byte_offset`, `Parsed::Object`'s per-key `key_byte_offset`,
   `Parsed::Constructor`'s `tag_byte_offset`, and the public `line_col_of(input, byte_offset)`.
   loft's lexer is **line:col-char native** (`Position{line, pos=char column}`, no byte
   offset). Reproducing byte-identical offsets is therefore a real sub-task, not free.

2. **Driving quirks.** The lexer lexes `-` as its own token (numbers can't be negative), and
   `true`/`false`/`null` as identifiers — so the JSON parser combines the sign and dispatches
   the literals. It drives via `peek()` + `cont()`; each `LexResult` carries the token-start
   `Position`.

**Refinement (2026-07-17, probed) — numbers read the raw lexeme, not the lexer's number token.**
Empirically, loft's number lexer is unsuitable as the *value* source for JSON: (a) it overflows
silently above `i64::MAX` — `9223372036854775808` and `18446744073709551615` both lex to
`Integer(0)`, garbage — where the old scanner produced the exact `Number(f64)`; (b) it reports
the sign as a separate `Token("-")` (JSON `LexConfig` gained `-`, since it was missing); (c) it
discards the raw lexeme needed for an exact `str::parse::<f64>()`. So the JSON parser reads a
number's **raw lexeme** from the input (the old `parse_number` char-scan) via the token's byte
offset, and re-syncs the lexer past it. This is byte-identical to the old scanner AND yields the
integer-shape classification Phase 3 needs (no `.`/`e` and fits i64 → `Int`) directly from the
lexeme — the lexer's overflow-prone `Integer`/`Long` value is not consulted. The lexer's genuine
value-add is thus **structure + strings (the JSON-escape reader) + identifiers**, exactly the
"one genuine addition" (`\uXXXX`/surrogates) the gap analysis named.

**Decision (owner, 2026-07-17) — Option 1: keep the byte-offset model.** A small `ByteMapper`
(a line-starts table built once from the input + a UTF-8-width walk along the target line)
converts the lexer's `(line, char-col)` back to the exact absolute `byte_offset` the old
scanner produced. This keeps `ParseError`, the `Object`/`Constructor` offset fields, and
`line_col_of` **byte-for-byte unchanged**, so the Phase-0 golden stays valid and **no consumer
changes** — Phase 2 is a true drop-in replacement. Option 2 (migrate the public model to
line:col) was rejected: it would rewrite the golden, change `ParseError`'s public shape, and
ripple into every diagnostic consumer for no functional gain, violating the minimal-blast-
radius + NO-HYBRID discipline. The mapper is throwaway-free (it stays; it is genuinely how the
lexer-driven parser reports positions).

## Falsification points / risks

- **String escapes** — `\uXXXX` + surrogate-pair combining is the highest correctness
  risk vs loft's string lexer; the corpus must include astral-plane + surrogate cases.
- **Number acceptance (not strictness)** — by decision, loft is lenient (reuses its own
  number allowances; not a JSON validator), so the risk is *rejecting valid JSON*, not
  *accepting non-JSON*. The one known gap — uppercase `E` / `e+` exponents — is closed in
  Phase 1b. Corpus must include exponent variants (`1E5`, `1e+5`, `1e-5`) to pin acceptance.
  (Any consumer relying on the old scanner's *rejection* of a loft-number form would be a
  behaviour change — the Phase-0 differential surfaces it; expected to be none.)
- **Error parity** — `json_errors()` and consumer diagnostics rely on the current line/col
  + context snippet; Phase 2 must reproduce (or the corpus pins) the error text/positions.
- **Lenient dialect + `Parsed::Ident`/`Constructor`** — the enum JSON round-trip and
  `Dialect::Lenient` paths must survive the swap; include them in the corpus.
- **Performance** — the hand-rolled scanner is byte-level; loft's lexer is char-based
  (`Peekable<IntoIter<char>>`). The normal ser/deser pass **must not regress** (owner
  constraint); Phase 2 benches the typed parse vs the current path.

## Composition matrix — Stage A

This is a behaviour-preserving re-tokenization (Phase 0's corpus IS the matrix: every JSON
shape → identical `Parsed`, except the one intended int-preservation change). The new
composition surface is the `Parsed::Int` case, matrixed at the number-reading sites in
Phase 3. No other new value/type/op.

## Cross-arc dependencies

- **@PLN102 arc-E H5** — the trigger; H5 closes when this ships (or stays open pointing here).
- Consumers to keep green: registry (`registry_index`, `registry_advisories`), `rpc`,
  `ir_schema`, `database/snapshot`, `database/structures` — all read `Parsed`.

## See also

- [lib-audit.md](../102-stability-contract/lib-audit.md) § H5 — the trigger.
- `src/json.rs` (the scanner to retire) · `src/lexer.rs` (the lexer + `Mode` enum) ·
  `src/native.rs` (`json_parse`, `populate_struct_from_jsonvalue`, the extractors).
- Tracker: [@PLN109](https://github.com/loft-lang/plans/issues/109).
