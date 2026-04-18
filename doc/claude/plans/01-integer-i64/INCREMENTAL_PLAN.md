# Incremental Phase 2+4 landing plan

Status: **devised 2026-04-18** — breaks the coupled Phase 2+4 effort into
8 landable increments, each completable in a single session (<400 min)
without leaving the codebase in an inconsistent state.

**Progress (2026-04-18)**:
- **2a Done** (`8aee716`) — wide-limit-to-Long + `u32` stdlib alias.
- **2b Done** (`9940f57`) — Op*Long Nullable opcodes; Phase 1 G-hybrid
  fully covers long arithmetic.
- 2c, 2d, 2e, 2f, 2g, 2h — remaining.

## Guiding insight

The coupling (Phase 2 and Phase 4 breaking if split) came from the
naive approach: "widen `integer` to Long at parse time → collides
with `long` overloads in stdlib".

**Escape hatch**: widen ONLY when `integer limit(lo, hi)` has bounds
outside i32 range.  Unbounded `integer` and narrow-bounded `integer
limit(...)` keep Type::Integer (current i32 behaviour).  Wide-bounded
integers (bounds > i32::MAX) promote to Type::Long.  No stdlib
overload collisions because wide-bounded is a NEW shape that didn't
exist before.

This defers the "widen unbounded integer" decision until after the
smaller pieces prove out the architecture.

## The 8 increments

### 2a — "Wide limit promotes to Long" + `u32` stdlib alias

Scope:
- `src/parser/definitions.rs::parse_type` — when `type_name == "integer"`
  with `limit(lo, hi)` and either `lo < i32::MIN+1` or `hi > i32::MAX`,
  return `Type::Long` instead of `Type::Integer`.
- `default/01_code.loft` — add `pub type u32 = integer limit(0, 4_294_967_294);`
  (size clause optional; Long's 8-byte storage absorbs u32's range).
- Probes: u32 round-trip, RGBA math at boundary, `u32 not null`.

Delivers: **Phase 3 u32** + partial Phase 2 (wide bounds work).  No
breaking changes.  No opcode changes.

Budget: **120-180 min**.

### 2b — Op*Long Nullable opcodes

Scope:
- `default/01_code.loft` — add `Op{Add,Min,Mul,Div,Rem}LongNullable`
  opcodes (5 new).
- Grow `OPERATORS` array (260 → 265).
- Regen fill.rs.
- `src/parser/operators.rs::rewrite_outer_arith_to_nullable` — add
  Long cases.
- Tests: `(long_a * long_b) ?? default` for each op.

Delivers: **Phase 1 G-hybrid completion for long arithmetic**.  Post
2a, any wide-bounded integer (promoted to Long) also benefits.

Budget: **60-90 min**.

### 2c — Unbounded `integer` → `Type::Long`

Scope:
- `src/parser/definitions.rs::parse_type` — unbounded `integer`
  returns `Type::Long` (after the `has_limit` check).
- `default/01_code.loft` stdlib sweep: DELETE `fn f(both: long) ->
  long` for every function where `fn f(both: integer) -> integer` also
  exists.  Canonical list: `abs`, `min`, `max`, `round`, `sign`,
  `floor`, `ceil`, etc. (~30 overload pairs).
- Update `Op*Long` references in stdlib to use `integer` instead (they
  now mean the same thing).
- `pub type integer size(4)` → `size(8)` (documentation fix).

Delivers: **Phase 2 arithmetic + storage widening for unbounded integer**.
After 2c, `x: integer = 3_000_000_000` works (was: silent wrap).

Budget: **240-360 min** — the stdlib sweep is the bulk.

### 2d — Op*Int opcode deletion

Scope:
- All Op*Int arithmetic opcodes are now unused (parser never dispatches
  to them after 2c).
- Delete from `default/01_code.loft`: `OpAddInt`, `OpMinInt`, `OpMulInt`,
  `OpDivInt`, `OpRemInt`, `Op*IntNullable` (my Phase 1 additions
  redundantly added these; they move to Long now).
- Delete from `src/ops.rs`: `op_*_int` functions.
- Delete from `src/fill.rs`: the handlers.
- Reclaim ~10 opcode slots.
- Regen fill.rs.

Delivers: **Phase 5 opcode reclamation** (partial — Op*Long siblings
now canonical).

Budget: **120-180 min**.

### 2e — `long` / `l` deprecation warnings

Scope:
- `src/lexer.rs::ret_number` — when `l` suffix is seen, emit
  `Level::Warning` "deprecated; use plain integer (now i64) in 0.9.0+".
- `src/parser/definitions.rs::parse_type` — when `long` keyword is
  seen in a type position, emit warning.
- Add `#[allow_deprecated_long]` pragma to suppress in scope
  (deferred — not strictly needed yet).
- Tests: `c54b_long_type_deprecated`, `c54b_l_literal_deprecated`.

Delivers: **Phase 4 deprecation warnings** (user-facing).  Users still
can write `long` / `10l` during transition.

Budget: **60-120 min**.

### 2f — `--migrate-long` source rewriter

Scope:
- `src/migrate_long.rs` (new): AST-aware source rewriter.  Walk a
  `.loft` file; replace `long` in type positions with `integer`;
  replace `Nl` literals with `N`.  Preserve identifiers containing
  "long" (e.g. `long_value`).
- `src/main.rs` — CLI flag `--migrate-long <path>`.
- Dry-run mode: `--dry-run` prints diffs without writing.
- Tests: migrate fixtures with various shapes (type positions,
  literals, identifiers-to-preserve).

Delivers: **Migration tool** enabling users to prep their code.
Independent of 2c — can land before or after.

Budget: **180-300 min**.

### 2g — Stdlib / lib / tests sweep

Scope:
- Run `loft --migrate-long` on `default/*.loft`, `lib/*.loft`,
  `tests/*.loft`, `tests/scripts/*.loft`, `tests/docs/*.loft`.
- Fix any site the tool can't handle (rare).
- Update test expectations where source line numbers shifted.
- Once stdlib is clean, DELETE the `long` type declaration from
  `default/01_code.loft`.
- Delete remaining `Op*Long` opcodes (they share implementation with
  Op*Int — after 2d we kept Long; now at cleanup, pick the canonical
  set).

Delivers: **Phase 4 complete**.  `long` type and `l` suffix removed
from stdlib.  Users get hard error at 1.0.0.

Budget: **240-360 min**.

### 2h — Spec + initiative close

Scope:
- `doc/claude/LOFT.md` — new "Arithmetic safety" section; remove
  `long` from type reference.
- `doc/claude/CHANGELOG.md` — 0.9.0 entry (C54 landed) + 1.0.0 entry
  (`long` removed).
- `doc/claude/PROBLEMS.md` — C54 marked closed.
- `doc/claude/CAVEATS.md` / `INCONSISTENCIES.md` — update relevant
  entries.
- `doc/claude/QUALITY.md` — strike C54 sprint entry.
- `doc/claude/RELEASE.md` — 0.9.0 progress.
- Move initiative dir to `plans/finished/`.

Delivers: **Phase 6 spec + close-out**.

Budget: **120-180 min**.

## Ordering + interdependencies

```
     ┌─→ 2a (wide-limit → Long, u32 alias) — STANDALONE
     │        │
     │        └─→ 2b (Long Nullable opcodes) — completes Phase 1
     │
     │   2c (unbounded integer → Long, stdlib sweep) ─────→ 2d (delete Op*Int)
     │        │                                                    │
     │        └────────────────────────────────────────────────────┤
     │                                                             ↓
     └─→ 2e (deprecation warnings) + 2f (migrate tool) ─→ 2g (stdlib sweep) ─→ 2h (spec + close)
```

- 2a is FULLY independent — can ship now.
- 2b depends on 2a OR can ship independently (it's just opcode growth).
- 2c is the BIG one — depends on 2a for the promotion pattern, but
  adds the unbounded rule.  Atomic with stdlib-overload sweep.
- 2d follows 2c (dead-code deletion).
- 2e + 2f can be done in parallel before/during 2g.
- 2g depends on 2c + 2e + 2f.
- 2h last.

## Total budget

Summed: **1140-1740 minutes** (19-29 hours).  Break across 6-10
sessions.  Each increment is independent progress; no session leaves
an inconsistent state.

## Decision now

Pick one of:

- Execute **2a** in this session (~120-180 min): delivers u32 working,
  proves the architecture.  Then stop, let the next session pick up
  2b or 2c.
- Execute **2a + 2b** (~180-270 min): delivers u32 + completes Phase 1
  Long Nullable.  Ambitious for one session but possible.
- Execute **2a + 2b + 2e** (~240-390 min): adds deprecation warnings.
  Pushing the session limit.

Recommendation: **2a this session**.  Concrete user-visible delivery.
Low risk.  Leaves momentum for a dedicated 2c+2d+2g session later.
