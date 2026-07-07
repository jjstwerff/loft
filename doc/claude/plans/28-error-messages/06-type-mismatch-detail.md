<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 6 — Concrete type-mismatch messages

Status: **delivered 2026-07-07** — see § Delivered.  The atomic sequence
and templates below are the original design, retained for context.

## Delivered (2026-07-07)

This phase's premise ("today's messages use generic phrasing like 'type
mismatch'") was **stale**: the messages had already evolved (the
`validate_convert` direction fix — 6.1's `Type::name` — and phase 5) to
name **both sides + the operation** at nearly every site.  An audit
(each 06.A site tested on `--interpret` + the golden corpus) found most
templates were effectively already met, and two of the spec's target
cases actually contradict loft's design.  The genuine gaps, fixed at
their chokepoints:

1. **Argument index** (6.6, `parser/mod.rs`).  A call-arg mismatch was
   `expected E, got G on call to F`; it now names the position:
   `expected integer, got text on argument 2 of call to add`.  One-line
   context enrichment (`nr` was already in scope).
2. **Match-pattern type mismatch** (6.10, `parser/control.rs`).  A
   pattern whose type can never match the subject (`match x {"hi"=>…}`
   with `x: integer`) **silently exited 0** with a dead arm.  It now
   errors `cannot match integer against pattern of type text`.  The
   `incompatible` detection already existed as a #493 codegen-width
   guard; this upgrades it from silent-recovery to a reported error
   while keeping the width-safe dead-`false` recovery.
3. **Struct extra-field cascade** (`parser/objects.rs`).  `Player{name,
   level:5}` (unknown `level`) emitted the correct `Unknown field
   Player.level` **then a 5-error cascade** (`Expect token }` / `;` / …)
   because the orphaned `: value` was left unconsumed.  The error branch
   now consumes the value → **one** clean error (6 → 1).

Baselines 06 / 30 / 34 regenerated + locked by `baselines_are_locked_in`;
full suite green on both backends; the match change surfaced **no**
previously-silent dead arms anywhere in the suite.  No runtime path was
touched (all changes are parse-time diagnostics / recovery), so `make
bench` is definitionally flat — not re-run.

### Intentionally NOT changed (design, not bugs)

- **Missing struct field** (spec case 33): `Player{name:"Bob"}` omitting
  `health` is **legal by design** — an omitted field takes its zero /
  default value (LOFT.md:261 `= expr` default; DESIGN_DECISIONS.md:1189
  "`S{}` gives the zero value").  Forcing a "missing field" error would
  break intended partial construction.  The plan's case-33 expectation
  was wrong; behaviour left as-is.
- **Format-spec on wrong type** (spec case 40): `{x:d}` on a text value
  is silently ignored.  Format specs are **freeform by design**
  (LOFT.md:1621 treats `{r:128,…}` as a spec); per-type spec validation
  is a new subsystem, not a contained message fix — out of scope.
- **Phrasing-only rewrites** (assignment / operator / return /
  struct-field templates): the current messages already name both sides
  + the operation (`No matching operator '/' on 'text' and 'integer'`,
  `Cannot assign integer to field S.name of type text`, `cannot change
  type from text to integer`).  Rewriting them to the exact template
  strings is lateral churn with fixture cost and no user gain; left.
- **`= note:` decl-pointer lines** (the templates' second lines): the
  diagnostic system has no secondary-note channel (phase 5 deferred that
  renderer too).  Decl-pointers would need that infra; deferred, keeping
  the shipped inline style.

## Goal

Every type error names **both sides**, the **operation**, and (for
function calls) the **argument index**.  Today's messages use
generic phrasing like "type mismatch" or "wrong arg type"; phase 6
rewrites them to:

```
error: cannot assign vector<i32> to variable of type text
  --> game.loft:42:9
   |
42 |     name = scores
   |     ^^^^^^^^^^^^^^
   = note: 'name' was declared as text on line 8
```

```
error: argument 2 of fn 'fight' has wrong type
  --> game.loft:88:14
   |
88 |     fight(player, 100)
   |                   ^^^
   = note: expected reference<Enemy>, got integer
   = note: 'fight' is defined at battle.loft:12
```

This is purely a rendering / message-construction change — spans
(phase 1), pretty rendering (phase 2), and note lines (phase 5)
do all the heavy lifting.  Phase 6 is the largest "polish" patch:
many small message rewrites, each with a fixture.

## Decision 06.A — message templates

Every type error obeys the template:

```
<verb> <expr-summary> <preposition> <expected> [from <found>]
```

Concrete templates per site:

| Site | Template |
|---|---|
| Assignment | `cannot assign <found-type> to variable of type <expected-type>` |
| Function arg | `argument <N> of fn '<name>' has wrong type` (+ note: `expected <T>, got <U>`) |
| Operator | `operator '<op>' cannot apply to <lhs-type> and <rhs-type>` |
| Field access | `type <T> has no field '<name>'` (suggestion via phase 5) |
| Method call | `type <T> has no method '<name>'` (suggestion via phase 5) |
| Iterator | `cannot iterate over <found-type>; expected vector / hash / sorted / index / text` |
| Match arm | `cannot match <expr-type> against pattern of type <pattern-type>` |
| Return | `cannot return <found-type> from fn '<name>' declared to return <expected>` |
| Struct literal | `field '<f>' of struct <S>: expected <T>, got <U>` (per field, not whole struct) |
| Format spec | `format spec '{:<spec>}' is not valid for type <T>` |

Note lines add the "where was the expected type declared" pointer
when cheap (we already have the def's `Position`).

## Decision 06.B — type rendering

`Type` enum has a `Display` impl today (`src/data.rs`).  Phase 6
audits it for legibility:

| Type | Today | Want |
|---|---|---|
| `Type::Int` | `Int` | `integer` |
| `Type::Long` | (removed in @PLAN01) | n/a |
| `Type::Single` | `Single` | `single` |
| `Type::Float` | `Float` | `float` |
| `Type::Boolean` | `Boolean` | `boolean` |
| `Type::Text` | `Text` | `text` |
| `Type::Vector(Int, _)` | `Vector(Int, _)` | `vector<i32>` (uses size annotation) |
| `Type::Hash(K, V, _)` | `Hash(K, V, _)` | `hash<V[K]>` |
| `Type::Sorted(K, V, _)` | similar | `sorted<V[K]>` |
| `Type::Reference(T, _)` | `Reference(T, _)` | `reference<T>` |
| `Type::Tuple(elems)` | debug repr | `(T, U, V)` |
| `Type::Function(args, ret)` | debug repr | `fn(T, U) -> V` |

This matches loft surface syntax — what the user writes is what
they see in errors.  Phase 6 renames `Display` to a new
`fn render_user(&self) -> String`, leaves `Debug` untouched (still
useful for IR dumps).

### Decision 06.B-followup — a single source of truth

`render_user` lives in `src/data.rs` next to `Type`.  Every
diagnostic site consumes it, never the `Debug` impl.  The
formatter (`src/formatter.rs`) and the documentation generator
(`src/gendoc.rs`) already render types in surface syntax — phase 6
makes them call `render_user` so all three (errors, formatter,
docs) share one implementation.

## Steps

### 6a — `Type::render_user`

New method.  Covers every variant.  Unit tests in `src/data.rs`
under `#[cfg(test)]` cover each variant.  Formatter and gendoc
switch over.

### 6b — Site rewrite, batch 1: assignments + return

Sites in `parser/expressions.rs` (assignment) and
`parser/control.rs` (return).  Rewrite ~6 messages per file.  Each
rewrite is a one-line message change plus a `note:` entry pointing
at the variable / fn declaration.

`tests/error_messages/baseline_pretty/`: cases 5, 11 regenerated.

### 6c — Site rewrite, batch 2: function calls + struct literals

Sites in `parser/control.rs::parse_call` and
`parser/objects.rs::parse_struct_literal`.  These are the
bulk-message sites — most user-visible type errors fire here.

Cases 6, 31, 33, 34 regenerated.

### 6d — Site rewrite, batch 3: operators + iterators

Sites in `parser/operators.rs` and `parser/collections.rs`.

Cases 32 regenerated.  (Case 32 = `for x in 5 { … }`.)

### 6e — Site rewrite, batch 4: match + format

Sites in `parser/control.rs::parse_match` and the format-string
type-check (lexer hands off to expression parsing for
`f"{x:spec}"`).

Cases 30, 40 regenerated.

### 6f — Test pass

After each batch, `cargo test error_messages` re-runs.  No
unrelated test should churn — phase 6 only touches messages, not
control flow or types.  Any non-`error_messages/` test that diffs
indicates a hidden coupling (e.g. a test grepping for "type
mismatch") and is patched in the same batch.

`make bench` re-run after batch 4: zero expected delta — these are
diagnostic strings, no runtime change.  Bound: ≤ 0.5 % drift.

## Atomic landing sequence

| # | Step | Test |
|---|---|---|
| 6.1 | **Shipped 2026-05-13.**  Extended the existing `Type::name(&self, &Data) -> String` (already used at user-facing error sites) to cover every variant explicitly — Unknown, Null, Void, Never, Boolean, Float, Single, Character, Integer (default / byte / bounded), Keys, Iterator, Tuple, Function — instead of falling through to the Display fallback that lower-cased the debug format (e.g. `tuple([integer(...), text([])])`).  20 unit tests in `data::type_name_user_facing_tests` cover each variant.  Three pre-existing tests updated to assert the new cleaner format: `p140_vector_range_slice_reports_type_mismatch`, `quality_6d_keyed_collection_constructor_hint`, `par_worker_returns_generator`.  Did NOT add a separate `render_user` method — `name()` already played the role; one method, no duplication. | ✓ Unit + regression tests green |
| 6.2 | Switch `formatter.rs` to `render_user` | `tests/format.rs` green; if any test diffs, the diff is "Int" → "integer" style and is updated in the same commit |
| 6.3 | Switch `gendoc.rs` to `render_user` | `tests/doc_hygiene.rs` green; gendoc HTML diff is the same "Int" → "integer" style |
| 6.4 | Rewrite assignment-mismatch message + "declared on line N" note | Case 5 fixture: `.expect` regen shows `cannot assign vector<i32> to variable of type text` + decl note |
| 6.5 | Rewrite return-mismatch message | New synthetic fixture: `fn foo() -> integer { return "x" }` → `cannot return text from fn 'foo' declared to return integer` |
| 6.6 | Rewrite call-arg-mismatch (arg index + decl note) | Case 6 fixture: `argument 2 of fn 'foo' has wrong type` + `expected …, got …` + decl note |
| 6.7 | Rewrite struct-literal field-mismatch (per-field, not whole struct) | Case 33: missing field message names the field; case 34: extra field message names it; both regenerated |
| 6.8 | Rewrite operator-type-mismatch | New fixture: `"x" / 5` → `operator '/' cannot apply to text and integer` |
| 6.9 | Rewrite iterator-not-iterable | Case 32 fixture: `cannot iterate over integer; expected vector / hash / sorted / index / text` |
| 6.10 | Rewrite match-arm-type-mismatch | Case 30 fixture |
| 6.11 | Rewrite format-spec-type-mismatch | Case 40 fixture: `format spec '{:int}' is not valid for type text` |
| 6.12 | Audit non-`error_messages` tests for hidden "type mismatch" greps | Each batch's PR runs `cargo test`; any churn outside `tests/error_messages/` is fixed in the same PR or rolled back |
| 6.13 | Re-run `make bench`; expect ≤ 0.5 % drift | Bench gate |

## Acceptance

- `Type::render_user` covers every variant; formatter and gendoc
  switched over.
- Every site listed in 06.A's table emits the new message shape.
- All 9 type-error baseline cases (5, 6, 11, 30-34, 40)
  regenerated.
- Zero non-`error_messages` tests changed by phase 6 (any change
  is a hidden coupling and gets a fix in the same PR).
- `make ci` green.
- `make bench` ≤ 0.5 % drift vs phase 5.

## Risks

| Risk | Mitigation |
|---|---|
| Massive fixture churn obscures the actual content of each PR | Batch 1-4 are separate commits; each commit's diff shows only the message rewrite + the matching `.expect` regen.  Reviewer (phase 7's checklist) reads them per-batch. |
| `Type::render_user` and `Display` drift (someone adds a variant only to one) | `#[deny(unreachable_patterns)]` on the match in `render_user` plus a test that exhaustively matches every variant.  Same trick as `src/data.rs` already uses for `op_code` checks. |
| Hidden test couplings to old message phrasing | Phase 0's baseline corpus and phase 1's grep audit list every test that asserts on diagnostic text.  Any other coupling surfaces in batch 1 and is fixed before batch 2. |
| Localisation gets harder if messages become longer | Plan-07's out-of-scope statement excludes localisation; that's a future plan.  English-only stays. |

## Cross-reference — null-vs-not-null in type messages (per 4h)

Phase 4h adds `Level::Hint` for "field read 47× with no defense
— consider `not null`."  The hint is a runtime/usage observation,
not a type mismatch.  But the hint's text references `not null`
as a type annotation — phase 6's `Type::render_user` MUST
reproduce `not null` consistently with what 4h's hint shows
(both quote the keyword exactly: `not null`, not `notnull` or
`Not Null`).  Phase 6 owns the type-string rendering used by
both type errors AND 4h hints; one source of truth (per
06.B-followup) keeps the two layers in sync.
