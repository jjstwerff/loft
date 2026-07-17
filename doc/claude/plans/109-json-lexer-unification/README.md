<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 109 — Unify JSON tokenization on loft's own lexer

Tracker: [@PLN109](https://github.com/loft-lang/plans/issues/109) · `subject:loft` · `status:future`

## Status

Open — design ready, no implementation. Triggered by @PLN102 arc-E lib-audit **H5**
(JSON integers > 2⁵³ silently round through f64, corrupting even a known-type `integer`
field on deserialize). Rather than patch the duplicate scanner, retire it: drive JSON
tokenization from loft's own lexer, which already distinguishes int from float. **H5 is
fixed as a consequence.**

## Goal

Delete the hand-rolled Rust JSON scanner in `src/json.rs` and re-drive `crate::json`'s
parse on **loft's own lexer** (`src/lexer.rs`, via a JSON mode), keeping `Parsed` as the
stable output interface so consumers barely change. One lexer to maintain; integer JSON
values preserved to i64 on the typed path.

## Effort + design

- **Effort:** MH (a subsystem's tokenizer, wide consumer surface, but interface-preserving).
- **Design:** ✓ (phasing + gap analysis below; risks enumerated).
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

## Gap analysis — loft's lexer vs JSON's grammar

loft's lexer has a `Mode` enum (the extension point) and the numeric tokens we want, but
JSON's grammar differs in three places the JSON mode must own:

| Concern | loft lexer today | JSON needs | Design |
|---|---|---|---|
| **Strings** | `string()` / `string_nested()` — `{expr}` interpolation, loft escapes | `\uXXXX` (+ surrogate pairs), `\/`, standard escapes, **no** interpolation, `{` literal | JSON mode string reader: no interpolation, JSON escape set incl. `\uXXXX` (port `parse_hex4` logic) |
| **Numbers** | `Integer(u32,bool)` / `Long(u64)` (unsigned!), `Float` needs a `.`, `Single` `f`-suffix | signed i64 ints, `1e5` exponent-without-dot, strict (no leading zero, digit after `.`) | JSON mode number reader: leading `-` is part of the number; integer-shaped + fits i64 → i64, else f64; JSON grammar strictness |
| **Structure** | `{ } [ ] : ,`, `true`/`false`/`null` | same | reuse as-is |

The sign + i64 width is the one place JSON numbers don't map 1:1 onto loft's existing
tokens (loft's `Integer`/`Long` are unsigned magnitudes; loft lexes `-` separately) — the
JSON mode's number path produces a signed i64/f64 directly. **No i128:** an integer token
beyond i64 falls back to f64 (a documented ceiling — loft cannot present > i64 anyway).

## The interface that does NOT change — `Parsed`

`Parsed` (Null/Bool/Number/Str/Ident/Array/Object/Constructor) is the output type every
consumer reads. Keep it; add an integer case (`Parsed::Int(i64)`). Only ~10 sites match
`Parsed::Number` (`ir_schema`, `rpc`, `registry_index`, `snapshot`, `registry_advisories`,
`native`, `database/structures`) — each gains an `Int` arm (read i64 directly; in a float
context, `Int`→f64). Consumers of the JSON *stdlib* (`native.rs` extractors + the typed
`populate_struct`) are the ones that gain exact integers. This keeps the blast radius at
the number-reading sites, not the whole registry/rpc/snapshot surface.

## Sub-arcs — the migration steps

| Step | Item | Status |
|---|---|---|
| 0 | **Characterize** — a golden corpus of current `crate::json` output over every consumer's real inputs (JSON stdlib tests, registry index, RPC messages, IR schema, snapshots, advisories) + a throwaway differential harness (old vs new → identical `Parsed`, except the intended int-preservation). | Open |
| 1 | **JSON mode in loft's lexer** — strings (`\uXXXX`+surrogates, no interp), JSON number grammar (int/float, exponents, strictness, signed i64), `{}[]:,`, literals. Unit-tested against the edge cases (the risk rows). No consumer touched. | Open |
| 2 | **Reimplement `parse()` / `parse_with()`** on the JSON mode → same `Parsed` tree + `Int` preserved. Prove byte-identical vs the Phase-0 corpus (except int-preservation). Preserve error line/col/context + `json_errors()`. | Open |
| 3 | **Update `Parsed::Number` consumers** for the `Int` case; the typed `populate_struct_from_jsonvalue` reads `Int`→i64 into an `integer` field exactly — **H5 fixed**. Each consumer re-verified. | Open |
| 4 | **Delete** the hand-rolled scanner (`parse_number`/`parse_string`/`parse_value` byte-scanning) + the differential scaffold. Confirm `crate::json` is now a thin layer over loft's lexer. **No hybrid remains.** | Open |
| 5 | **Verify + freeze** — full suite both backends; corpus → regression suite; H5 typed-integer golden added; STDLIB.md JSON number semantics (int→i64, float→f64, > i64 ceiling) + lib-audit H5 → DONE. | Open |

## Falsification points / risks

- **String escapes** — `\uXXXX` + surrogate-pair combining is the highest correctness
  risk vs loft's string lexer; the corpus must include astral-plane + surrogate cases.
- **Number grammar** — JSON forbids leading zeros, requires a digit after `.`, allows
  `1e5`; the JSON mode must enforce JSON grammar, not loft's lenient rules. Corpus: the
  strict-reject cases the current scanner rejects.
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
