<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 5 — Did-you-mean suggestions

Status: **delivered 2026-07-07** — all seven candidate-scoped sites in
05.A now suggest.  See § Delivered below for the exact end state; the
atomic sequence lower down is the original design and is retained for
context.

## Delivered (2026-07-07)

An audit (matrix over all 7 sites × cap-passing / cap-failing typos on
both `--interpret` and the golden corpus) found the work was **not** "5
missing wires" — 6 of 7 sites were already wired.  The real gaps were
three root causes, each fixed at its chokepoint:

1. **Systemic distance cap too strict** (`suggest_similar_capped`,
   `src/diagnostics.rs`).  The old `min(2, name.len()/4)` cap only
   reached Levenshtein-2 at 8+ chars, so it silently dropped every
   4–7-char **transposition** (`naem`→`name`, `Bleu`→`Blue`,
   `reuslt`→`result`) — the single commonest real typo.  The
   variable-suggestion site had already worked around this by using the
   uncapped `suggest_similar` (see its comment).  Fix: 1–3 chars never
   suggest (generic placeholders `T`/`K`/`V`, coin-flip pairs); 4+ chars
   get the full distance-2 ceiling.  This one change lit up field-access,
   method, both enum-variant syntaxes, function-name, and struct-type
   suggestions at once.
2. **Struct-literal type site unwired** (`parser/objects.rs`, the
   `unknown type '…'` branch).  Now calls `suggest_type_name`
   (`Plyer{…}` → "did you mean 'Player'?").
3. **Qualified enum-variant `Enum::Typo` silently recovered as null**
   (`parser/objects.rs`, the #493 recovery — exit 0, printed `null`,
   hid the typo).  Now reports `unknown variant Enum::Typo — did you
   mean '…'?` on pass 2 (enum-scoped candidates via `suggest_field_name`,
   since variants live in the enum's attributes) while keeping the
   null recovery that avoids the #493 self-reference codegen crash.

Coverage: golden cases 07 (field), 08 (method), 09 (struct-type,
`.loft` updated to define a near struct so the suggestion is exercised),
10 (enum-variant `::`, exit 0→1) regenerated + locked by
`baselines_are_locked_in`.  Method case 08 also gained a real suggestion
once the cap allowed distance-2 (`lengt`→`len`).  Full suite green on
both backends; no collateral churn in the other 40 golden cases.

Not done (intentionally): the `= note:` sub-line renderer (5.2/5.12) —
suggestions render **inline** on the error line instead, which meets the
user goal with less machinery; and a `LOFT_NO_SUGGESTIONS` toggle (5.4)
was never wired (no consumer has asked to suppress).  Both remain open
if a need surfaces.

## Goal

Every `<thing> not found` diagnostic in the second pass appends
`did you mean <best>?` when a near-match exists in the right
candidate scope.  The `suggest_similar` helper already exists
(`src/diagnostics.rs:181`); phase 5's job is to wire it in at every
applicable site with the **right** candidate set — not all global
names.

## Decision 05.A — candidate scoping rules

Bad suggestions are worse than no suggestions.  Per-site rules:

| Diagnostic | Candidate set | Why |
|---|---|---|
| Variable not found | locals + arguments in current fn | function-scoped |
| Function not found | all `n_*` defs visible from current file (respecting `use` and `pub`) | global, but visibility-filtered |
| Method not found on type `T` | methods defined on `T` and its interfaces | type-scoped |
| Field not found on struct `S` | fields of `S` only | struct-scoped |
| Struct type not found | all struct typedefs visible from current file | type-scoped |
| Enum variant not found | variants of the named enum | enum-scoped |
| Format-string `{name}` capture not found | locals + arguments | function-scoped |

Suggesting `min` when the user typed `len` because edit distance is
2 is wrong — both are functions, but they belong to wildly
different concepts.  Edit distance ≤ 2 is a necessary filter, not
a sufficient one; **the candidate set is the primary filter**.

## Decision 05.B — distance cap

`suggest_similar` returns the closest match within edit distance
≤ 2.  Phase 5 tightens the cap dynamically:

```rust
let cap = std::cmp::min(2, name.len() / 4);   // 25 % of length, max 2
```

For a 3-letter name, the cap drops to 0 — too short for distance to
be meaningful, no suggestion.  For an 8-letter name the cap stays
at 2.  For a 16-letter name the cap stays at 2 (we never go above).

This is a single change in `suggest_similar`; sites do not change.

## Steps

### 5a — Audit existing not-found sites

`grep -n "not found\|unknown\|undefined" src/parser/*.rs src/typedef.rs src/scopes.rs`

Catalogue in `5a-sites.md` (new):

| Site | Candidate set today | Candidate set in 5b |
|---|---|---|
| `parser/expressions.rs:??` "Variable X not found" | none (no suggestion) | locals + args |
| `parser/control.rs:?? parse_call` "Function X not found" | none | visible global fns |
| `parser/fields.rs:?? .` "Field X not found on struct S" | none | fields of S |
| `parser/fields.rs:?? .` "Method X not found on T" | none | methods of T |
| `parser/objects.rs:?? S {}` "Type X not found" | none | visible struct typedefs |
| `parser/control.rs:?? match` "Variant X not found in enum E" | none | variants of E |
| `default/03_text.loft` `f"{name}"` capture | (parsed in lexer; sites in `parser/expressions.rs`) | locals + args |

### 5b — Wire candidate sets

For each site, build the candidate slice from the data already
on hand:

```rust
// Variable not found:
let candidates: Vec<&str> = self.vars.iter()
    .filter(|v| v.scope_visible_at(here))
    .map(|v| v.name.as_str())
    .collect();
if let Some(suggestion) = suggest_similar(&unknown, &candidates) {
    diagnostic_with_suggestion(self.lexer, Level::Error,
        format!("Variable '{unknown}' not found"),
        format!("did you mean '{suggestion}'?"));
}
```

The `_with_suggestion` variant adds the suggestion as a `note:`
line in the pretty renderer (a second `DiagEntry` at `Level::Note`,
sharing the same position as the parent error).

Phase 2's renderer already supports note lines via cascading
entries; phase 5 just plumbs the `Note` level through.

### 5c — Renderer note-line support

Add `Level::Note` to `diagnostics.rs`:

```rust
pub enum Level { Debug, Note, Warning, Error, Fatal }
```

`Note < Warning` so it doesn't trigger a non-zero exit code.  The
pretty renderer prints note entries indented under their parent:

```
error: Variable 'naem' not found
  --> game.loft:12:9
   |
12 |     print(naem)
   |           ^^^^
   = note: did you mean 'name'?
```

Pairing: a `Note` entry whose `(file, line, col)` exactly matches
an `Error` entry that immediately precedes it is rendered as a
sub-line of that error (the `= note:` form).  Stand-alone notes
render as their own block.

### 5d — Suppression flag

Test fixtures churn when suggestion thresholds change.  Add an env
var `LOFT_NO_SUGGESTIONS=1` which disables wiring in 5b — the
diagnostic still fires, the note line is suppressed.  The test
harness sets this when running the wider suite that doesn't care
about suggestion stability.

`tests/error_messages.rs` does *not* set it — that suite is the
suggestion-quality regression target.

### 5e — Tests

- Cases 7 (`naem`), 8 (`lengt`), 9 (`Plyer`), 10 (`Bleu`) from
  phase 0 are the golden suggestions.  `.expect` regenerated to
  include `note: did you mean …?`.
- New `tests/suggestions.rs` covers anti-cases:
  - Single-letter typo (`x` vs `y`) — no suggestion (cap = 0).
  - Distant name (`foo` vs `printbar`) — no suggestion.
  - Same name in different scope (`x` exists in a sibling fn) — no
    suggestion (candidate scope filter).
  - Method on wrong type (`text.push` vs `vector.push`) — no
    cross-type suggestion.
- `make ci` green; `LOFT_NO_SUGGESTIONS=1 cargo test` also green
  (proves the toggle works for harness use).

## Atomic landing sequence

| # | Step | Test |
|---|---|---|
| 5.1 | Run audit, write `5a-sites.md` with per-site candidate-set spec | Manual review |
| 5.2 | Add `Level::Note` between `Debug` and `Warning`; update orderings | Unit test: `Level::Note < Level::Warning`; `Level::Note < Level::Error`; `Diagnostics::level()` ignores notes (max ignores Notes) |
| 5.3 | Tighten `suggest_similar` cap to `min(2, name.len() / 4)` | Unit test: 3-char name → no suggestion at distance 1; 8-char name → suggestion at distance 2; 16-char → cap stays at 2 |
| 5.4 | Add `LOFT_NO_SUGGESTIONS=1` toggle that short-circuits suggestion calls | Integration test: same fixture with toggle off shows note, toggle on does not |
| 5.5 | Wire variable-not-found candidate set (locals + arguments visible at the site) | Fixture: `print(naem)` with `name` in scope → "did you mean 'name'?" |
| 5.6 | Wire function-not-found (visible global fns; respects `use` and `pub`) | Fixture: `lengt(s)` where `length` exists → "did you mean 'length'?"; verify a `pub fn` from another file is suggested only when used |
| 5.7 | Wire field-not-found (struct-scoped) | Fixture: `p.naem` on `Player { name, … }` → "did you mean 'name'?" |
| 5.8 | Wire method-not-found (type-scoped — methods of `T` and its interfaces) | Fixture: `t.lengt()` on text → "did you mean 'length'?"; cross-type anti-test (`text.push()` where only `vector.push` exists) — no suggestion |
| 5.9 | Wire struct-type-not-found | Fixture: `Plyer { name: "x" }` where `Player` exists → suggestion |
| 5.10 | Wire enum-variant-not-found | Fixture: `Color::Bleu` where `Blue` exists → suggestion |
| 5.11 | Wire format-string capture-not-found (`f"{naem}"`) | Fixture: typo'd capture in format string → suggestion using local-scope candidates |
| 5.12 | Pretty renderer pairs `Note` entries that share `(file, line, col)` with the immediately-preceding `Error` as `= note: …` | Unit test: `[Error@(f,l,c), Note@(f,l,c)]` renders Note as indented sub-line of Error; `[Error@(f,l,c), Note@(g,m,n)]` renders Note as standalone block |
| 5.13 | Anti-suggestion regression tests | `tests/suggestions.rs`: short-name suppression, distant-name suppression, sibling-scope suppression, cross-type suppression — each asserts no `note:` line in rendered output |
| 5.14 | Regenerate baseline_pretty/ for cases 7, 8, 9, 10 | Golden test |

## Acceptance

- All sites in `5a-sites.md` wired with their proper candidate
  set.
- `Level::Note` exists and renders correctly in pretty mode.
- 4 baseline cases (7, 8, 9, 10) gain `note: did you mean …?`
  output.
- `tests/suggestions.rs` covers the no-suggestion edge cases.
- `LOFT_NO_SUGGESTIONS=1` suppresses notes site-wide.
- `make ci` green.

## Risks

| Risk | Mitigation |
|---|---|
| Suggestion churn breaks unrelated tests on every stdlib rename | Test harness sets `LOFT_NO_SUGGESTIONS=1` by default; only `tests/error_messages.rs` and `tests/suggestions.rs` see them. |
| Wrong candidate scope produces misleading suggestions ("did you mean a function called `x`?" when the user typed a struct field `x`) | 05.A's per-site scoping is the safety belt; never use a wildcard "all names" set. |
| Levenshtein cost on large candidate lists | Stdlib has ~300 fn names; the pass runs only on a not-found error (rare).  No hot path. |
| `Level::Note` insertion changes ordering in `Diagnostics::lines()` and breaks other consumers | `lines()` is an iteration order, not a sort.  Notes are appended after their parent error; existing consumers see them as additional entries.  Audit existing callers (test harnesses, formatter): all just print/concat. |

## Cross-reference — `Level::Note` reuse for 4e.2 / 4h hints

Phase 4e.2 (undefended-fault-site warning) and 4h (`not null`
field-reminder hint) both consume the `Level::Note` machinery
this phase introduces — each warning ends with `note:` lines
naming the three defense patterns (length check / `??` /
`if x != null`).  Land 5.2 (`Level::Note` between Debug and
Warning + ordering rules) BEFORE 4e.2 to avoid 4e.2 inventing
its own ad-hoc note rendering.  4e.2 / 4h then reuse the
note-line emission pattern documented in 5.12.

Concretely: 4e.2 plumbs the diagnostic as

```rust
diagnostic_with_notes(self.lexer, Level::Warning,
    format!("`v[i]` may produce null on out-of-bounds with no defensive check"),
    vec![
        "guard with `if i < len(v) { ... }` before indexing".into(),
        "or accept null with `v[i] ?? <fallback>`".into(),
        "or follow with `if x != null { ... }` to catch the null".into(),
    ]);
```

The renderer cascades each note as `= note: …` indented under
the warning line — same shape as phase 5's "did you mean …?"
suggestions.  No new diagnostic infrastructure needed for
4e.2 / 4h beyond what 5.2 ships.
