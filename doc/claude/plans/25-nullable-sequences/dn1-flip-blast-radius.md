<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN25 DN1 default-flip — blast-radius measurement (2026-07-01)

The mandatory pre-flip count (RESUME Step 3 / Step 4). The flip = `IntegerSpec.not_null`
default `false → true` (plain scalars become non-null; `τ?` is the nullable form). Two distinct
"blast radii": the **`.loft` sweep** (what user/test code must migrate) and the **internal
reconciliation** (compiler work). Measured on the corpus: stdlib `default/` (6), `lib/` (13),
`tests/` (631).

## A. The `.loft` sweep — SMALL, and the FOUNDATION is clean

**Controlled surface (stdlib + lib) — essentially ZERO.** Every `==null` / `=null` match in
`default/*.loft` is in a COMMENT (no code site). `lib/` has exactly ONE genuine assignment —
`lib/code.loft:129 self.cur_def = null` — and its field is ALREADY annotated `cur_def: i32?`.
So the foundation needs **no migration**.

**Corpus-wide genuine `(N-Decl)` targets — ZERO.** The explicit pattern `x: <scalar> = null`
(an explicitly-typed scalar declared null) has **0** occurrences across `default/ lib/ tests/`
(integer family, text, bool/char/float/single). Struct field default `: <type> = null`: **0**.
Bare inferred `x = null` (inference governs nullability — NOT a flip break): 3.

**Test-side surface — the bounded sweep (dozens of files, each a one-char `?`):**
- `scalar == null` / `!= null`: ~132 occurrences across ~34 `tests/scripts` files. Most are
  INTENTIONAL null-round-trip tests on currently-nullable plain scalars (e.g. `U8N { x: null };
  … x == null`) — after the flip the field needs `u8?`. Only 2 of the 34 already use `?`.
- Scalar-returning-null fns (`-> <scalar>` body with `else { null }` / `return null`): ≤21 files
  (upper bound; genuine subset smaller — some return ref/vector, some already `?`). 94 total
  `else {null}`/`return null` occurrences corpus-wide.

**Estimate:** the genuine sweep is the union of the scalar-`==null` and scalar-return-null test
files — **on the order of 30–50 test files**, each a small `?` annotation. The **stdlib + lib
foundation needs ZERO changes** — the single most important blast-radius fact, since those must
stay releasable.

## B. The internal reconciliation — the real implementation cost

Flipping `not_null` is NOT a pure type-check toggle; only 8 sites read `.not_null`, but:
1. **Range / sentinel reservation.** `usable_min/usable_max(nullable)` reserve a narrow-int
   sentinel based on nullability. The flip changes the usable range of narrow integers (e.g.
   `u8` 0..254→0..255), which must reconcile with the `Optional(Integer)` sentinel (i64::MIN /
   the reserved narrow value) so `integer?` still has a null encoding.
2. **Bare-`null` rejection.** The landed `(N-Store)` checks (return / index / field / typed-store)
   fire on `Type::Optional → non-Optional`. They do NOT catch a bare `Value::Null` returned/stored
   into a not_null scalar (`fn f() -> integer { … null }`). The flip needs the `not_null` flag to
   gate that bare-Null rejection — additional enforcement beyond the Optional path.
3. **Redundant-null-check warning** (`parser/vectors.rs:296`, `expr_not_null`): flipping fires it
   on every `int == null` — the noise the RESUME flagged. This is what makes the test-side
   `== null` sites surface as warnings.

## C. How to pin the precise number

The static survey BOUNDS the sweep (foundation clean; ≤~50 test files). The EXACT count is the
suite failure count from the gated flip: add `LOFT_PLN25_DN1` (implies OPT+DN3), flip `not_null`
default gated, `find_problems.sh` with the gate ON → the failures ARE the blast radius, each
fixed one-char (`?`) or migrated. That is step (d)+(e); the measurement here says it is SAFE to
start — the foundation is clean and the sweep is bounded and test-local.
