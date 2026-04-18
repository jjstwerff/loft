# Phase 0 — `not null` enforcement audit + G/G′ decision

Status: **open** — gating every subsequent phase.

## Why this is Phase 0

The C54 design (`QUALITY.md § 479-557`) offers two semantic fixes for
arithmetic overflow and divide-by-zero:

- **C54.G** — trap (runtime error) on overflow / div-zero.
- **C54.G′** — null on overflow / div-zero, composes with `??` and
  `?? return`.

G′ is strictly the stronger UX win when it works.  It only works if a
null cannot silently reach a `not null` slot.  Trap → null-on-overflow
is a backward-compatible relaxation later; null → trap is breaking.
So the audit MUST land before either implementation.

## Pre-audit findings (preliminary, documented in this file)

An initial exploration of the enforcement surface turned up concrete
gaps.  The audit converts these from suspicions into committed
verdicts via probe fixtures.

| Site | Current state | Hole? |
|---|---|---|
| `??` operator (`src/parser/operators.rs:592-670`) | Recognises field-marked `not null` only.  Does NOT see arithmetic-overflow-null.  Line 597: `if self.expr_not_null` gates warning, but `expr_not_null` is set from field-access analysis, not from arithmetic result types. | **YES — ??-composition doesn't fire on overflow null** |
| Struct field writes (`src/parser/objects.rs::handle_field:1078-1411`) | Compile-time `convert()` at line 1399; no runtime null-check at OpSetInt emission. | **YES — `v.field = nullable_expr` silently writes `i32::MIN`** |
| Struct constructor positional+named (`src/parser/objects.rs:1078-1140`) | Same `convert()` funnel; same miss. | **YES** |
| Function parameter passing (`src/parser/control.rs::parse_call:2680-2950`) | Compile-time `convert()` at line 2911; no runtime null-check at call entry. | **YES — `foo(a * b)` for `a*b` overflow silently passes `i32::MIN`** |
| Return-type narrowing — tuple (`src/parser/control.rs::parse_return:2419-2435`) | Explicit T1.7 error: "cannot assign null to 'integer not null' element" on tuple literals. | NO (held) |
| Return-type narrowing — non-tuple (`parse_return:2438-2439`) | `validate_convert("return", ...)` → `can_convert()`; no runtime narrowing op. | **YES — `return a * b` for a `not null` return silently returns `i32::MIN`** |
| Array / hash index (`src/parser/vectors.rs` + `src/parser/collections.rs`) | No null-check on index before `OpGetVector`. | **YES — `v[a*b]` silently reads an out-of-range slot or fails at runtime without clear diagnostic** |

Provisional conclusion based on these findings: **at least 5 holes are
open** → the design's fallback applies: **ship C54.G (trap) first**;
G′ becomes a later relaxation once the `not null` contract is
tightened.

The audit phase still needs to run the probes to turn "at least 5"
into an exact count and to catch holes not anticipated above (e.g.
method-style call receivers, iterator bounds, parallel-loop indices).

## Probe fixtures to build

Create `doc/claude/plans/01-integer-i64/probes/` with the following
fixtures.  Each probe constructs a program that would route a
nullable integer into a `not null` slot without explicit `??` or
narrowing.  Each fixture's expected status is listed; the audit
verdict is assigned by running the fixture.

| # | File | Shape | Expected |
|---|---|---|---|
| 00 | `probe_00_baseline_overflow_produces_null.loft` | `a = i32::MAX; a + 1` — confirm the sentinel mechanism is live | Produces `i32::MIN` / null |
| 01 | `probe_01_field_write_nullable_into_not_null.loft` | `struct S { v: integer not null }` + `s.v = a + b` where `a + b` overflows | **HOLE** if silent; CLEAN if error/trap |
| 02 | `probe_02_field_write_default_initialised.loft` | `struct S { v: integer not null = 0 }` + later `s.v = nullable_expr` | **HOLE** if silent |
| 03 | `probe_03_field_write_nested_path.loft` | `struct A { b: B }; struct B { v: integer not null }` + `a.b.v = nullable_expr` | **HOLE** if silent |
| 04 | `probe_04_call_arg_nullable_to_not_null.loft` | `fn foo(x: integer not null)`, call with `foo(a + b)` overflow | **HOLE** if silent |
| 05 | `probe_05_chained_call_returning_nullable.loft` | `fn bar() -> integer { ... }` (nullable), call `foo(bar())` where `foo` takes `not null` | **HOLE** if silent |
| 06 | `probe_06_return_narrowing_no_null_check.loft` | `fn foo(x: integer) -> integer not null { x }` called with null | **HOLE** if silent |
| 07 | `probe_07_return_nullcoalesce_discharges.loft` | `fn foo(x: integer) -> integer not null { x ?? 0 }` — happy case | CLEAN (held) |
| 08 | `probe_08_nullcoalesce_on_arithmetic.loft` | `x = a * b ?? default` where `a * b` overflows | **HOLE** — the decisive probe; `??` should catch but doesn't today |
| 09 | `probe_09_index_key_null.loft` | `v[nullable_expr]` and `v[i32::MIN]` | **HOLE** if silent |
| 10 | `probe_10_ref_forwarding_preserves_non_null.loft` | `fn foo(x: &integer) { }; fn outer(y: &integer not null) { foo(y); }` — does the forward preserve? | Likely CLEAN but confirm |

**Critical probe**: `probe_08_nullcoalesce_on_arithmetic.loft`.  If
`??` does NOT catch arithmetic-overflow-null, G′ is unsafe (the
`??`-composition argument from QUALITY.md § 537 evaporates).
Conversely, if by some miracle it does catch, G′'s case holds.

## Decision procedure

Run every probe.  For each, record verdict:

- **CLEAN** — the null is either caught (diagnostic or error) or
  propagates observably without silent coercion to `not null`.
- **HOLE** — the null silently reaches a `not null` slot and is
  treated as if it were non-null.

Decision rule:

- **0 holes** → G′ is safe.  Phase 1 ships `01-checked-arith.md`
  with G′ semantics.
- **≥1 holes** → G ships first.  File each hole as a follow-up in
  `00.1-hole-*.md` sub-phases for future tightening; do NOT block
  Phase 1 on closing them.  Phase 1 ships `01-checked-arith.md`
  with G semantics.

Given the preliminary findings above, expect the ≥1 branch.  That is
the SAFE default per the design.

## File + line references for the audit

The audit code-reading tour:

| Area | File | Lines |
|---|---|---|
| `??` op semantics | `src/parser/operators.rs` | 592–670 |
| `expr_not_null` field-access tracking | `src/parser/` (grep for `expr_not_null`) | — |
| Field write compile-time check | `src/parser/objects.rs::handle_field` | 1078–1411 (convert at 1399; set_field_no_check at 1409) |
| Runtime field write opcodes | `src/fill.rs` | `OpSetInt`, `OpSetShort`, `OpSetByte` handlers |
| Function call arg check | `src/parser/control.rs::parse_call` | 2680–2950 (convert at 2911) |
| Return narrowing — tuple case | `src/parser/control.rs::parse_return` | 2419–2435 |
| Return narrowing — non-tuple | `src/parser/control.rs::parse_return` | 2438–2439 |
| Null sentinel values | `src/data.rs::I32` | 32 |
| Storage read/write | `src/store.rs::{get,set}_{int,short,byte}` | 1115, 1124, 1153, 1167, 1184, 1194 |
| Arithmetic null-producing sites | `src/ops.rs` | 531 `op_add_int`; 313 `op_add_long`; the `checked_int!` macro at 33–52 |

## Deliverables

- `doc/claude/plans/01-integer-i64/probes/probe_00.loft` … `probe_10.loft`
- An **"Audit result"** appendix to this file:
  - Per-probe verdict (HOLE / CLEAN).
  - Hole count.
  - Decision: G or G′.
  - Rationale paragraph.
- `Status: done` flip on this file.
- `01-checked-arith.md` renamed or retitled to reflect the chosen
  option (e.g. "Phase 1 — C54.G trap on overflow").

## Budget

**120–180 minutes**.  Bail out after 180 min: ship G based on the
preliminary findings above; open holes as follow-ups.  Do not chase
an inventory past diminishing returns.

## Non-goals for Phase 0

- Implementing either G or G′.  Phase 0 decides; Phase 1 implements.
- Fixing any holes that surface.  Each hole becomes a follow-up
  sub-phase AFTER G ships.
- Touching `.loftc` format, storage layout, opcodes, stdlib.

## Success criteria

1. All 11 probes committed.
2. Each probe classified with a committed verdict.
3. Audit-result appendix committed.
4. Phase 1 opens with the chosen option named in its file + title.
