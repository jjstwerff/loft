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

## A′. CORRECTION — the stdlib is NOT clean (the gated flip found the real targets)

Building the gated flip (`LOFT_PLN25_DN1`, the bare-`null`→non-null-scalar rejection extending
`n_store_violation`) and turning it ON immediately FAILED stdlib load with precise diagnostics:
`default/01_code.loft` has **null-PROPAGATING functions** declared `-> integer`/`-> single`/
`-> float` that `return null` (e.g. `min`/`max`/`clamp`: `if !a || !b { return null }`, and the
`*Nullable` arithmetic operators). My grep missed these — they `return null` INSIDE the fn body,
not via the `else { null }` / `= null` patterns I searched. **These are the genuine first DN1
targets, and they are the DN1↔DN3 intertwining the RESUME predicted.**

**The design fork (must decide before migrating them):** under DN1 a plain `integer` is non-null,
so `min(a, b)` receives non-null inputs and its `if !a || !b { return null }` is DEAD. Two options:
1. **Drop the null-propagation** — `min`/`max`/`clamp` stay `-> integer` (non-null); delete the
   `if !a || !b { return null }`. No caller ripple. Loses legacy null-propagation (fine under DN1
   where inputs can't be null; gate-OFF would change behaviour, so gate it).
2. **Make them `-> integer?`** — honest about the (now-unreachable-under-DN1) null return, but
   ripples to EVERY caller of `min`/`max` (must discharge). This is the DN3 "biggest blast radius".
Recommend **option 1** (drop the dead null-propagation under DN1) — it matches the model (non-null
inputs ⇒ non-null result) and avoids the ripple. The `*Nullable` ops (`OpDivIntNullable`) genuinely
fit-fail (`/0`) → those DO want `-> τ?` (true DN3).

## A″. RESOLVED — option 1 realized via STDLIB EXEMPTION (gate-OFF-safe)

Decision: **option 1 (drop the dead null-propagation), realized by EXEMPTING the stdlib**, not by
editing the `.loft`. Removing `min`/`max`'s `if !a || !b { return null }` from `default/01_code.loft`
broke `tests/scripts/17-min-max-clamp.loft` (asserts `!min(null, 5)` — null-propagation IS
gate-OFF-load-bearing), so a pure `.loft` removal changes the DEFAULT — not allowed during the
GATED phase. Instead: the DN1 bare-`null` rejection now EXEMPTS `STD_SOURCE`
(`self.data.source != crate::data::STD_SOURCE`). Rationale: the stdlib's null-propagation is DEAD
under DN1 (its scalar params are non-null, so the bare `null` never flows), so not rejecting it is
correct, not a hack. The stdlib stays byte-identical gate-OFF (`17-min-max-clamp` passes); under DN1
the stdlib LOADS and only USER code gets the rejection. At step (f) — DN1 default — the stdlib gets
properly migrated (`min`/`max` cleaned, the test updated) and the exemption removed.

## IF-branch absorbed-null — FIXED (match-arm remaining)

**`if`/`else` with an absorbed bare-null branch is now caught under DN1** (`parse_if`, control.rs):
when exactly one branch yields a bare `null` (detected by `branch_yields_null`, which descends
`Block`/`Insert`/`Span`/`If`) and the other is a non-null scalar `τ`, the if-expression's result
type widens to `Optional(τ)` — and the existing DN3 `(N-Store)` then forces the caller to declare
`τ?` or discharge. Matrix (both backends): `if b {5} else {null}` / `if b {null} else {5}` / the
nested `else if c {6} else {null}` all REJECT into a non-null return; `-> integer?`, no-null,
discharged (`?? 0`), both-null, and a HEAP-reference null branch (`else { null }` of a `Node`
return — stays nullable, no Optional) all ACCEPT. gate-OFF byte-identical; suite green.

**MATCH arms — DONE for scalar + enum/struct.** A `=> null` arm now widens the match result to
`Optional(τ)` under DN1 (verified both backends, `dn1-if-match-branch-null.loft`):
- **enum/struct match** — widened in `parse_match`'s arm loop, checking the USER `arms`
  (`a.tp == Null` or `branch_yields_null(a.code)`), NOT the lowered value. Crucial finding: an
  exhaustive match synthesises an unreachable `else OpConv*FromNull` default, so the value-level
  check would falsely widen EVERY match — `nonull` (no null arm) must NOT widen, and doesn't.
- **scalar match** — widened at the dispatch via `dn1_widen_branch_null` (value-level): a scalar
  match's `_` arm is USER-written (no synthesised default), so the value-level check is safe there.
- **REMAINING (rare): vector/tuple match** null arms — the dispatch returns at control.rs (vector,
  tuple) are not yet wrapped (the value-level helper may hit a synthesised default for those; needs
  the arm-loop check like enum if a consumer surfaces it).

## ⚠️ (historical) KNOWN GAP (narrowed) — the IF/MATCH-branch absorbed-null

The DN1 rejection FIRES on every DIRECT bare-`null` store (verified gate-ON, `n_store_violation`
instrumented): explicit `return null` (`parse_return`), a typed-scalar store (`x: integer = null`,
caught after `change_var`), and **field-construct** (`S { a: null }` → "null cannot be stored into
the field of the non-null scalar type `integer`"). My earlier note WRONGLY listed field-construct as
broken — it works; I had misread the diagnostic.

The ONE remaining gap is an ABSORBED null in an if/match branch: `fn f() -> integer { if b { 5 }
else { null } }`. `parse_if` (control.rs:1926) parses the `else` with the THEN branch's type
(`integer`) as expected, so the bare `null` is coerced to the typed-null sentinel at the branch and
the if-expression types as plain `integer` — the return then sees a non-null integer, nothing to
reject. It is a MISSING REJECTION, not corruption (the null flows as the sentinel; reads as null).
**Fix (a focused future increment): under DN1, when one if/match branch is a bare `null` and the
other is a non-null scalar `τ`, the unified result type is `Optional(τ)`** (then the existing DN3
`(N-Store)` catches it at the return). Touches `parse_if` (the then-null path at ~1923 + a new
else-null path) and the match-arm unification; must preserve gate-OFF and handle nested-if. Land it
before the `.loft` sweep, or accept it as a known under-enforcement for the first DN1 cut.

## B′. The gate + enforcement — LANDED (gated, opt-in)

`LOFT_PLN25_DN1` added (implies DN3⊃OPT); `n_store_violation` extended with the DN1 branch: a bare
`Value::Null` into a non-Optional SCALAR target (`is_non_null_scalar`: Integer/Text/Bool/Float/
Single/Character — heap types stay nullable) is rejected "declare it `τ?`". gate-OFF byte-identical;
suite green. Gate-ON the stdlib doesn't load yet (the null-propagating fns above must migrate first)
— expected for an in-progress flip; the gate is opt-in.

## C′. MEASURED — the precise blast radius (gate-ON suite run, 2026-07-01)

`LOFT_PLN25_DN1=1 cargo nextest run` (enforcement complete): **2557 passed, 24 failed** (well below
the ~30–50 estimate). Of the 24, ONE is the pre-existing chrome env test (`html_asyncify`); the
other 23 are the blast radius, and they trace to a SMALL, CONCENTRATED set of `.loft` sources —
NOT 30–50 scattered files. The failing TESTS cascade from a few shared libraries:
- **`lib/lexer.loft` — ~23 distinct sites** (the dominant source). The lexer's tokenizer fns return
  `null` from scalar returns (`integer`/`text`/`single`/`float`). It is loaded by the multiplayer
  v2/v3/v5, viewer_markdown, native, and most `wrap::*` tests — so fixing this ONE file clears the
  bulk of the 23 failures.
- **the `web-0.2.1` registry lib** (`src/web.loft`) — a few sites (the multiplayer websocket tests).
- **~5 test scripts**: `tests/docs/{10-sorted,11-index,13-file}.loft`, `tests/scripts/{08-functions,
  10-match}.loft`, plus `38_import_unknown_file`, `81-iterator-protocol`, `p54_nested_struct_enum`.

So the SWEEP is ~30 sites across ~7 files, dominated by one library. Each is a scalar-returning-null
fn (or a nullable scalar field) → migrate to `-> τ?` / `τ?` and discharge at call sites, OR (where a
scalar fn returns null purely as an error signal) rework to a non-null return. **A found enforcement
bug fixed along the way:** an if-WITHOUT-else synthesises a `null` else for recovery, which the DN1
widening mis-read as a nullable branch (`if_expr_without_else` failed "Cannot format type integer?").
Fixed with a `had_else` guard — the widening only fires for a REAL user `else`.

## C. How to pin the precise number

The static survey BOUNDS the sweep (foundation clean; ≤~50 test files). The EXACT count is the
suite failure count from the gated flip: add `LOFT_PLN25_DN1` (implies OPT+DN3), flip `not_null`
default gated, `find_problems.sh` with the gate ON → the failures ARE the blast radius, each
fixed one-char (`?`) or migrated. That is step (d)+(e); the measurement here says it is SAFE to
start — the foundation is clean and the sweep is bounded and test-local.
