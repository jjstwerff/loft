// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Plan 06 phase 4d — Tuple / Fn-ref struct field follow-ups

Four issues remained open after the phase-4d landing that made fn-refs
and tuples (including nested, with text / vector / record elements)
work as struct fields.  Each has a concrete fix path; this doc turns
the `PROBLEMS.md` "Fix path" pointers into implementation designs
ranked by effort and severity so the next session can pick them up
in priority order.

| ID | Issue | Severity | Effort | Status |
|----|-------|----------|--------|--------|
| ~~P193~~ | ~~Default-init for fn-ref / tuple struct field~~ | ~~High~~ | ~~XS~~ | **DONE** — `Type::Function` + `Type::Tuple` arms in `to_default`; regression guards `p4d_fn_ref_field_default_init`, `p4d_fn_ref_field_bare_default`, `p4d_tuple_field_default_init`. |
| **P196** | Tuple-of-fn-ref native codegen | Medium | **S** (~half day) | open |
| **P195** | Lexer ambiguity on `n.v.0.0` | Low | **S** (~half day) | open |
| **P194** | Tuple-field reassignment `p.v = (1, 2)` | Medium | **M** (~1–2 days) | open |

Remaining to close all three: **roughly 3 days.**

---

## P196 — Tuple-of-fn-ref native codegen

### Symptom

```loft
struct C { pair: (fn(integer) -> integer, integer) }
fn test() { c = C { pair: (dbl, 21) }; }
//                                       ^ native:
//   error: non-primitive cast: `(u32, DbRef)` as `i32`
```

### Root cause

`set_field_check`'s `Type::Tuple` arm dispatches a fn-ref element to
`OpSetInt4(ref, pos, TupleGet(tmp, i))`.  `TupleGet(tmp, i)` for a
`Type::Function` element resolves at native emit to `var_tmp.i`,
whose Rust type is `(u32, DbRef)`.  `OpSetInt4`'s `#rust` body does
`@val as i32`; rustc rejects the tuple cast.

Interpreter mode works because the fn-ref stack representation is
8B i64 d_nr + 12B DbRef, and `OpSetInt4` reads i64 from the top of
stack — which is the d_nr.  Only native breaks.

### Design

Add a dedicated runtime helper opcode `OpSetFnRef(v1: reference,
fld: const u16, val: function)`:

```loft
fn OpSetFnRef(v1: reference, fld: const u16, val: function);
#rust"{{let db = @v1; stores.store_mut(&db).set_i32_raw(db.rec, db.pos + u32::from(@fld), @val.0 as i32);}}"
```

The native body reaches into `.0` of the `(u32, DbRef)` tuple to get
just the d_nr.  Interpreter implementation in `src/fill.rs` pops the
20-byte fn-ref slot, takes the i64 d_nr's low 4 bytes, writes 4 bytes
to storage.

Then update `set_field_check` (the standalone `Type::Function` arm
AND the tuple-element `Type::Function` arm in `emit_set_one_element`)
to call `OpSetFnRef` instead of `OpSetInt4` when the source is
fn-ref-shaped.

For the standalone arm: keep the `Value::FnRef → Value::Int(d_nr)`
reduction path for the common case where the parser already produced
a bare `Value::Int(d_nr)`; add the `OpSetFnRef` route only when the
source IR is a `Value::Var` of fn-ref type or a `Value::TupleGet`
of fn-ref-typed element.

### Verification

1. New regression `p4d_tuple_field_with_fn_ref`:
   ```loft
   struct C { pair: (fn(integer) -> integer, integer) }
   fn dbl(x: integer) -> integer { x + x }
   fn test() {
     c = C { pair: (dbl, 21) };
     // reading the fn-ref out is a separate concern (P196b):
     // for now just verify SET compiles + the integer half works.
     assert(c.pair.1 == 21);
   }
   ```
2. Existing `p4d_fn_ref_as_struct_field` must still pass.

### Open question — read path

Reading a fn-ref OUT of a tuple element (`c.pair.0` returning a
fn-ref) needs a matching `OpGetFnRef` that inflates the 4-byte
storage to the 20-byte stack slot.  My current `get_val::Type::Tuple`
arm recursively calls `get_val` for each element, which already hits
the `Type::Function` arm I added (the `OpGetInt4` + `OpNullRefSentinel`
block).  Should work — but worth a regression test to confirm.

### Files touched

- `default/01_code.loft` — declare `OpSetFnRef`.
- `src/fill.rs` — interpreter implementation.
- `src/parser/mod.rs` — `set_field_check` and
  `emit_set_one_element` route through `OpSetFnRef`.
- `tests/issues.rs` — new regression.

### Effort

**S**.  One new opcode (declaration + interpreter + native via
#rust), two parser arms updated.  The declaration uses the same
shape as existing `OpSet*` opcodes so no special codegen support
is needed.

---

## P195 — Lexer ambiguity on `n.v.0.0`

### Symptom

```loft
struct Nested { v: ((integer, integer), (integer, integer)) }
fn test() {
  n = Nested { v: ((1, 2), (3, 4)) };
  a = n.v.0.0;          // parse error: float `0.0` token then `.`
}
```

### Root cause

`src/lexer.rs` greedy-reads `<digit>.<digit>` as one floating-point
literal token.  After `n.v.`, the lexer sees `0.0` and emits a single
`Float(0.0)` token, leaving the parser confused about the trailing
`.0`.

### Design

Two viable approaches; we recommend (b).

**(a) Context-aware lexing**: track the previous non-trivia token in
the lexer.  When the previous token was `.` (field-access dot), force
integer-only number reading on the next number.  Implementation: add
`prev_token: Option<TokenKind>` to the `Lexer` struct, special-case
`read_number` when `prev_token == Some(Dot)`.

  *Risk*: changing lexer state across the whole parse stream affects
  unrelated constructs.  Anything that currently tokenises `<expr>.0.5`
  intentionally as `expr` `.` `Float(0.5)` would silently shift to
  `expr` `.` `Int(0)` `.` `Int(5)`.  The risk is real but small —
  loft uses field-access dots only on identifiers, and there's no
  legitimate `expr.float-literal` construct in the grammar.

**(b) Parser-driven re-lex**: keep the lexer greedy; in
`parse_field` when the next token is a `Float` AND the field-access
dot was just consumed, split the float into two integer indices.

  Concretely: after `parse_field` reads the first `.<int>` (a
  `TupleGet` arg), if the next lookahead is `Float(f)` such that
  `f` is exactly representable as `<int>.<int>`, peek the float's
  raw text from the lexer's source span and re-tokenise as
  `Int(<head>) Dot Int(<tail>)`.  Push both back onto the token
  stream.

  *Risk*: re-tokenising mid-parse is invasive.  The simpler shape
  is to ADD a method `Lexer::has_integer_index_chain()` that's only
  called from `parse_field`'s tuple-access path; it scans the
  fractional digits as separate integer tokens and restores any
  unconsumed input on failure.

### Recommended approach

(b) with a dedicated `has_chained_integer_indices()` helper.  Scoped
to the tuple-index parser, no risk to unrelated lexing.  ~50 lines.

### Verification

1. New regression `p4d_tuple_field_chained_index`:
   ```loft
   struct N { v: ((integer, integer), (integer, integer)) }
   fn test() {
     n = N { v: ((1, 2), (3, 4)) };
     assert(n.v.0.0 == 1);   // currently fails to parse
     assert(n.v.0.1 == 2);
     assert(n.v.1.0 == 3);
     assert(n.v.1.1 == 4);
   }
   ```
2. Add a counter-test that checks `1.5` etc. still parse as floats
   in normal expression context.
3. Check `tests/scripts/*.loft` for any user-visible expression
   that combines `.<int>.<int>` (none expected, but worth a grep).

### Files touched

- `src/lexer.rs` — new `has_chained_integer_indices()` (or similar).
- `src/parser/operators.rs` — tuple-index path consumes the chain.
- `tests/issues.rs` — new regression + counter-test.

### Effort

**S**.  Half-day including tests and the counter-test sweep.  Only
the tuple-access parser path changes, so the blast radius is small.

---

## P194 — Tuple-field reassignment `p.v = (1, 2)`

### Symptom

```loft
struct Pair { v: (integer, integer) }
fn test() {
  p = Pair { v: (3, 4) };       // construction OK
  p.v = (100, 200);             // ← parser:
                                //   "Tuple destructuring requires
                                //    plain variable names"
}
```

### Root cause

Two parser paths converge on `(...)= ...`:

1. **Tuple destructuring** — `(a, b) = expr` declares two locals
   from a tuple-typed RHS.  Activated when the LHS is a parenthesised
   identifier list.

2. **Tuple field assignment** — `field = (...)` writes the parens-
   built tuple into a field via the existing `set_field_check::
   Type::Tuple` arm.  Should activate when the LHS is a field-access
   expression of tuple type.

The current parser sees `p.v = (100, 200)` and the tuple-destructuring
branch fires too eagerly: it interprets `p.v` as the start of a
parenthesised list, fails because `p.v` isn't a "plain variable
name", and aborts.

### Design

Two-phase look-ahead in the assignment parser.

**Phase 1 — peek the LHS.**  Read tokens until the `=`.  If the
collected LHS token sequence is a single chain like `<ident>(.<ident>)*`
(field-access, no parens), it's a *field assignment*.

**Phase 2 — branch.**  If field-assignment AND the field's declared
type is `Type::Tuple(...)` AND the RHS opens with `(`, route through
`set_field_check::Type::Tuple` (which already handles writes).
Otherwise fall through to the existing destructuring path.

Implementation sketch in `src/parser/expressions.rs`:

```rust
// In the assignment parser, before committing to destructuring:
if let Some(field_path) = self.try_peek_field_path() {
    let field_tp = self.resolve_field_type(&field_path);
    if matches!(field_tp, Type::Tuple(_)) && self.lexer.peek_token("(") {
        self.commit_field_assignment(field_path, field_tp);
        return;
    }
}
// Existing destructuring path …
```

The hard bit is `try_peek_field_path` — it must read tokens
non-destructively and rewind on failure.  Loft's `Lexer` has a
`Mode::Peek` that lets us look ahead without consuming; the existing
`peek_token` / `has_token` family already use it.  We need a
multi-token peek that stops at `=` or end-of-statement.

Alternative simpler approach: parse the LHS expression first
unconditionally as an expression, then branch on the resulting
`Value` shape:
- `Value::Var(_)` of tuple type → existing destructuring (renames the
  var) OR tuple-typed assignment (if RHS is tuple literal).
- `Value::Call(OpGetField, …)` of tuple type → field assignment.
- Parenthesised name list → destructuring declaration.

This requires the LHS-as-expression parser to handle parenthesised
name lists — which today only the destructuring branch handles.  A
small refactor: lift parenthesised-name-list parsing into the
expression parser as a special "tuple LHS" form, then dispatch on
the type of the resulting form at the `=` boundary.

### Verification

1. New regression `p4d_tuple_field_reassign`:
   ```loft
   struct Pair { v: (integer, integer) }
   fn test() {
     p = Pair { v: (3, 4) };
     p.v = (100, 200);
     assert(p.v.0 == 100 && p.v.1 == 200);
   }
   ```
2. Counter-tests must still pass:
   - `(a, b) = (1, 2)` declaring two locals.
   - `(a, b) = some_fn()` destructuring a return value.
   - `(a, b) = pair_var` destructuring a stored tuple.
   - Tuple compound assignment `(a, b) += (1, 2)` still rejected
     (existing `tuple_compound_assign_rejected` test).

### Files touched

- `src/parser/expressions.rs` — assignment-parser disambiguation.
- `tests/issues.rs` — new regression + a few counter-tests if not
  already covered.

### Effort

**M**.  1–2 days.  The disambiguation logic is small but the test
coverage matrix is wide (destructuring vs. assignment, named vs.
field LHS, simple vs. compound assignment, return-value vs. literal
RHS).  Each combination needs a regression test to lock the new
behaviour.

---

## Suggested execution order

1. **P193** (XS, High) — close the high-severity bug first.  30 min.
2. **P196** (S, Medium) — closes the only remaining tuple-element
   shape that breaks native compile.  Half day.
3. **P195** (S, Low) — quality-of-life; users today work around with
   inner-stash, but `n.v.0.0` is the natural syntax.  Half day.
4. **P194** (M, Medium) — biggest scope but lowest pain (the
   `Pair {…}` rebuild workaround is one line).  1–2 days.

After all four close, the tuple/fn-ref struct-field feature is fully
ergonomic and the four `Cannot write …` / `Cannot assign to field
…` / `Tuple destructuring requires…` / `non-primitive cast` blockers
no longer reach users.

---

## See also

- [PROBLEMS.md](../../../PROBLEMS.md) — the open-issue catalogue (193 / 194 / 195 / 196).
- [04-typed-input-output.md](04-typed-input-output.md) — phase 4d
  parent plan.
- `tests/issues.rs::p4d_*` — 10 regression tests covering the
  closed phase-4d cases (fn-ref, tuple, mixed, nested).
