# Phase 0 — `not null` enforcement audit + G/G′ decision

Status: **open** — gating every subsequent phase.

## Goal

Produce a written, committed decision: **C54.G (trap on overflow / div-zero)
or C54.G′ (null-on-overflow composing with `??` / `?? return`)**.

G′ is the stronger UX win (see QUALITY.md § 537 — "the `??` composition is
the decisive win").  It is only safe if loft's `not null` contract is
tight enough that a null cannot silently propagate into a context that
assumes non-null.  This phase audits that contract and decides.

## Why this is Phase 0, not Phase 1

The G vs G′ choice is irreversible at the semantic level:

- **Trap → null-on-overflow** is a backward-compatible relaxation later.
- **Null → trap** would break every program that relies on `??` catching
  arithmetic-overflow-null.

So G ships first only if the audit surfaces holes.  G′ ships first only if
the audit is clean.  Either way, the audit MUST precede the implementation.

## Scope of the audit

Walk every site where a nullable integer may reach a non-null context.
Three categories per QUALITY.md § 542-554:

### 1. Struct field writes

- `record.field = expr;` where `field` is declared `not null` and `expr`
  is nullable.
- Default-initialised fields: `struct S { x: integer not null = 0 }`
  — does the `x = expr` later path enforce non-null?
- Nested struct updates: `record.inner.field = expr` — does the path
  walk preserve enforcement?

Files to inspect: `src/parser/objects.rs` (struct construction +
field-write codegen), `src/state/codegen.rs::generate_set`,
`src/fill.rs::set_field_int` family.

### 2. Function parameters

- `fn foo(x: integer not null)` called with a nullable `integer` expr.
- Chained calls: `foo(bar())` where `bar()` returns nullable.
- `&T` forwarding paths (P176 lineage) — does the forward preserve
  non-null?

Files: `src/parser/control.rs::parse_call_diagnostic`,
`src/state/codegen.rs::call_emission`, `src/fill.rs::OpCallUser`.

### 3. Return-type narrowing + coercion

- `fn foo(x: integer) -> integer not null { x }` — does the compiler
  emit a narrowing check on return?
- `return x ?? default;` flow — does control-flow analysis recognise
  the `??` as discharging the null contract?
- Struct construction with an expression that might be null fed to a
  `not null` field via positional / named syntax.
- Array-index / store-key coercion: `v[null]` behaviour; today this
  likely silent-wrongs because the key is read as an integer and
  the sentinel value flows through unchecked.

Files: `src/parser/control.rs::parse_return`,
`src/parser/objects.rs` (constructor arg coercion),
`src/parser/fields.rs` (index coercion).

## Method

**Minimal probe fixtures** (one per audit category, N=6-10 total):

- `probe_01_field_write_nullable_into_not_null.loft`
- `probe_02_field_write_default_initialized.loft`
- `probe_03_field_write_nested_path.loft`
- `probe_04_call_arg_nullable_to_not_null.loft`
- `probe_05_chained_call_returning_nullable.loft`
- `probe_06_return_narrowing_no_null_check.loft`
- `probe_07_return_nullcoalesce_discharges.loft`
- `probe_08_index_key_null.loft`
- `probe_09_constructor_positional_null_to_not_null.loft`
- `probe_10_ref_forwarding_preserves_non_null.loft`

For each probe:

1. Construct a program where a nullable integer would flow into a
   `not null` slot WITHOUT an explicit `??` or narrowing.
2. Run the probe.  Expected: compile-time diagnostic OR runtime error.
3. Actual:
   - If the probe is caught (diagnostic or error) → GOOD, `not null` is
     holding at that site.
   - If the probe runs silently to completion with the nullable value
     stored/used as-if non-null → HOLE; G′ is not safe here.

Hole-count decides:

- **0 holes** → G′ is safe.  Phase 1 ships C54.G′.  Add the probe suite
  as permanent regression coverage.
- **1+ holes** → G ships first (safer default).  File each hole as a
  separate sub-phase (`00.1-hole-field-writes.md` etc.) to tighten the
  `not null` contract.  Once holes close, G′ becomes a later
  relaxation.

## Deliverables

- `doc/claude/plans/01-integer-i64/probes/` — the 6-10 probe fixtures
  and their status.
- An **"Audit result"** section appended to this file with:
  - Hole count and per-probe verdict.
  - The chosen option (G or G′) with rationale.
  - If G, a follow-up list of holes to tighten before G′ can come back.
- A `Status: done` flip on this file.

## Budget

**120-180 minutes for the audit.**  Bail-out: if the probes surface more
than 3 holes, stop probing, ship G, and open the hole-list as its own
initiative.  Continuing to chase holes post-threshold costs more than
landing the safe default.

## Non-goals for Phase 0

- Implementing either G or G′.  Phase 0 decides; Phase 1 implements.
- Fixing the holes if any surface.  That's per-hole sub-phases AFTER G
  ships.
- Touching the `.loftc` cache format or migration tools.  That's
  Phase 2.

## Success criteria

1. 6-10 probes written, all committed.
2. Each probe classified as HOLE / CLEAN with reasoning.
3. Written decision committed to this file.
4. Phase 1 plan file (`01-checked-arith.md`) opens with the chosen
   option named in its title and the audit result linked.
