<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cluster IV-Spacial — parser infinite loop on `spacial<X[k]>` types

**Severity:** parser infinite loop — `loft` process is unresponsive until `LOFT_TIMEOUT` fires (or user `kill -9`s).  No backend reaches its run phase.

**Affected probes:** 51 (canonical with `??`), 62 (no `??`, still hangs), 88 (integer key, hangs), 89 (bare declaration, hangs), 90 (Node without spacial field — PASS reference baseline).  See [Probe set Z](README.md#curated-probe-sets--for-fix-attempt-validation).

**Backend asymmetry:** BOTH backends hang — the loop is in the PARSER, which runs before any backend selection.

## Mechanism (verified to a partial level)

Probes 51 / 62 / 88 / 89 together establish:

1. **The hang fires on type DECLARATION alone** — probe 89 hangs with just `s: spacial<Point[name]> = []; println(...);` — no insert, no read, no `??`, no other use.  The infinite loop is in `spacial<>` type registration itself.
2. **Key type doesn't matter** — probe 88 (integer key) hangs identically to probe 51 (text key).  The bug isn't key-resolution-specific.
3. **`??` isn't required** — probe 62 hangs without any `??`.
4. **A regular struct doesn't hang** — probe 90 (Node with `name: text, x, y` but NO spacial field) parses fine.

So the loop is somewhere in `src/parser/` or `src/typedef.rs` where `spacial<>` types are registered into the database.

PLAN49 breadcrumb (`LOFT_TIMEOUT=8 LOFT_TIMEOUT_CLEAN_EXIT=1`) localises to `phase=parse fn=? file=…:0` — the breadcrumb's fn-tracker doesn't get past the first checkpoint, so we know the hang is during file-level type registration, before any user function gets entered.

## Reference probe — 90 (struct without spacial field, PASS)

```loft
struct Node { name: text, x: float not null, y: float not null }
fn main() { n = Node { name: "root", x: 0.0, y: 0.0 }; }
```

Parses and runs cleanly.

## Problem probe — 89 (bare spacial declaration, HANG)

```loft
struct Point { name: text, x: float not null, y: float not null }
fn main() {
  s: spacial<Point[name]> = [];
  println("never reached");
}
```

Parser hangs at line 0 (per breadcrumb) — never reaches main()'s body.

## The divergence

`spacial<Point[name]>` triggers a parser code path that doesn't terminate.  The struct `Point` itself parses fine (probe 90); only the `spacial<>` wrapping causes the loop.

## What we know vs. don't

| | Status |
|---|---|
| Hang fires during parse-phase, before any backend | ✅ Verified via PLAN49 breadcrumb |
| Hang fires on type-declaration alone, no operations needed | ✅ Verified — probe 89 |
| Key type (text vs int) doesn't matter | ✅ Verified — probes 51 (text), 88 (int) |
| `??` doesn't matter | ✅ Verified — probe 62 |
| Regular structs without spacial field parse fine | ✅ Verified — probe 90 |
| Exact source location of the loop | ✅ **Localised 2026-05-29** — `src/parser/definitions.rs:1548-1562`.  The `spacial` arm's hand-rolled `while !has_closing_angle { has_token(","); has_identifier(); }` loop never advances when the next token is `[` (the start of the key-spec) because none of `has_closing_angle` / `has_token(",")` / `has_identifier()` call `cont()` on a non-matching `[`. |
| Does the loop exist in other keyed-collection types (sorted/index/hash)? | ✅ NO — sorted/hash/index call `parse_fields(true, ...)` which knows how to eat `[fields]>`.  spacial was the only sibling using the hand-rolled loop. |

## Investigation tasks

1. ~~Confirm hang fires on bare declaration~~ — done (probe 89).
2. ~~Confirm key-type independence~~ — done (probe 88).
3. ~~**Locate the parser/typedef loop site**~~ — done 2026-05-29.  Site: `src/parser/definitions.rs:1548-1562`, the `"spacial"` arm of the keyed-collection match in `parse_type`.
4. ~~**Hypothesise & verify**~~ — done.  None of the three hypothesised options (depth cap / memoisation / counter bug) applied.  The actual cause was simpler: the hand-rolled scanner loop in the `spacial` arm didn't handle the `[fields]` token sequence; the lookahead helpers it called all bail without advancing when the next token is `[`.

## Fix surface

**LANDED 2026-05-29.**  The hand-rolled `while !has_closing_angle { has_token(","); has_identifier(); }` scanner was replaced with a conditional call to the same `parse_fields` helper that sorted/hash/index already use:

```rust
"spacial" => {
    if self.lexer.peek_token("[") {
        self.parse_fields(false, &mut fields);
    } else {
        self.lexer.closing_angle();
    }
    diagnostic!(
        self.lexer,
        Level::Error,
        "spacial<T> is planned for 1.1+; until then use sorted<T> or index<T> for ordered lookups"
    );
    Type::Unknown(0)
}
```

The `peek_token("[")` branch handles `spacial<X[name]>` (key-spec present); the `else` branch handles bare `spacial<T>` (no key-spec — required by the existing `tests/issues.rs::p22_spacial_diagnostic_names_milestone_and_substitute` test).

### Fix iterations

**Iteration 1 (2026-05-29) — parser-hang fix landed**
- Site: `src/parser/definitions.rs:1548-1572` (spacial arm of `parse_type`).
- Result on Set Z probe 51 + sibling probes:

  | Probe | Shape | Before | After |
  |---|---|---|---|
  | 51 | `spacial<Point[name]>` value-block via `??` | HANG | PARSE-ERR (clean diagnostic + exit) |
  | 62 | `spacial<X[k]>` without `??` | HANG | PARSE-ERR |
  | 88 | `spacial<X[int_key]>` | HANG | PARSE-ERR |
  | 89 | bare `s: spacial<X[name]> = []` declaration | HANG | PARSE-ERR |
  | 90 | Reference baseline (no spacial) | PASS | PASS |

- Set H baselines: **all PASS** (no regression).
- `tests/issues.rs::p22_spacial_diagnostic_names_milestone_and_substitute`: continues to PASS (the `else { closing_angle() }` branch preserves the bare-`spacial<T>` behaviour).
- Full `cargo test --test issues`: 681/681 pass.

Probes 51/62/88/89 now emit the "spacial<T> is planned for 1.1+" diagnostic and exit cleanly — they don't fully PASS the probe assertions because that would require spacial itself to be implemented (1.1+ work outside PLAN52's scope).  This cluster's exit criterion ("hang gone, clean diagnostic") is met.

**Effort**: XS (single arm in `parse_type`; done in ~30 min).
**Risk**: LOW — diagnostic surface unchanged, no semantic change to types that did parse.

## In-plan vs spinoff decision

Per the [in-plan vs spinoff policy](README.md#in-plan-vs-spinoff-policy), keep in-plan unless the investigation reveals 2+ additional parser-recursion sites elsewhere.  Currently spacial is a single site; the fix is one of Options A/B/C above.

If Investigation task 3 reveals the same loop pattern in `sorted<>` / `index<>` / `hash<>` declarations under specific conditions (currently passes for them, but the underlying machinery might be shared), spin off a PLAN53 for "parser type-registration recursion family" and move all the related probes there.

## Why this isn't a runtime cluster

Unlike clusters I/III/IV-Vec/etc. (which involve the `??` lowering and scope-pass interaction), IV-Spacial is purely in the parse phase.  It surfaced via PLAN52's cluster IV completion sweep (probe 51 was testing IV-Spacial's value-block behaviour with `??`, but the parser never got past type-registration to test the value-block path).  Even if the entire `??` lowering were removed, probe 89 would still hang.
