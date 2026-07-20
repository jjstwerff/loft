<!-- Copyright (c) 2026 Jurjen Stellingwerff -->
<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->

# Fix design — `??` with a type-incompatible default is unsound (SIGSEGV / silent corruption)

> **Status: DESIGN (2026-07-20).** A pre-freeze soundness bug found while building the
> Gate-2 behaviour corpus ([flip-gate-coverage-gaps.md](flip-gate-coverage-gaps.md)). Arc-E
> E2 territory (a MISSING error the type checker should fire — one-way door, add pre-flip).
> Small-safe-step, inert-first fix below.

## The bug

`τ? ?? d` (null-coalesce) requires the default `d` to be usable where a `τ` is expected.
loft's type checker does **not** enforce that: when `type(d)` is not assignable to `τ`, the
coalesce is built with mismatched branch types and the interpreter reinterprets one
representation as the other. `--native` rejects every such program at rustc (`E0308`), so
this is also a **backend divergence** (the #1 stability red-flag).

**Severity: HIGH** — the symptom spans the two worst classes:

| `LHS ?? default` | interpret | native | verdict |
|---|---|---|---|
| `ref? ?? int` (`hash<Row[k]>` `h[absent] ?? -1`) | **SIGSEGV** (int used as a DbRef pointer) | `E0308` | reject |
| `int? ?? float` (`n ?? 2.5`) | **`4612811918334230528`** (float bits read as int) | `E0308` | reject |
| `int? ?? text` (`n ?? "x"`) | garbage (text pointer as int) | `E0308` | reject |
| `text? ?? int` | silent empty | `E0308` | reject |
| `Row? ?? int` (nullable struct var) | silent `null` | `E0308` | reject |

## The invariant + the boundary (must NOT over-reject)

**Invariant:** `d` in `τ? ?? d` must be **assignable to `τ`** — exactly the rule assignment
and argument-passing already use (`Parser::convert`). Assignability is **one-directional**:
`float? ?? int` is VALID (int widens to float), but `int? ?? float` is NOT (float→int needs
an explicit cast). The fix must keep every currently-valid shape (verified both backends):

| Valid shape | why |
|---|---|
| same type (`int?/int`, `float?/float`, `text?/text`, `Row?/Row`) | trivially assignable |
| `float? ?? <int>` | int → float widening (a real numeric coercion) |
| integer spec differences (`u8? ?? 0`, `vec_of_u8[i] ?? i64`) | the `widen_ints` / `checked_narrow` paths already unify these |
| `?? null` and a `τ?`-typed default | the gate-2 `?? null` typing (result stays `τ?`) |
| `?? []` / `?? [99]` vector-literal default | the vector-default work-ref path |

## Root cause (the chokepoint)

`Parser::build_null_coalesce_default` (`src/parser/operators.rs:1602`) is the sole place a
default-form `??` resolves the default. It parses the default with the LHS base as a *hint*
(`rhs_type = self.parse_operators(rhs_hint, …)`, :1624) — but the hint is **advisory**: when
the default's actual type can't become the base, the code proceeds to pick a result type
(`widen_ints` :1698, `checked_narrow` :1721, else the base) and BUILD the coalesce anyway.
Nothing rejects a `rhs_type` that is not assignable to `lhs_type.base()`. That missing check
is the whole bug — no narrower, no wider.

(The sibling `build_null_coalesce_return` — `v ?? return …` — is not affected: `return` has
no value type to mismatch. Confirm during build, but the fix is one function.)

## The fix — one reject at the chokepoint

After `rhs_type` is known (:1624) and after the existing special-case bindings are computed,
add the soundness gate: **if the default is none of the already-handled sound shapes AND
`rhs_type` is not assignable to `lhs_type.base()`, emit a compile error and stop** — a clean
`diagnostic!(Level::Error, …)` (E1-coded, e.g. `coalesce-default-type-mismatch`), matching
the "No matching operator" reject style. It fires in the parser, so BOTH backends reject
identically (interpret stops before it can crash; native no longer needs rustc to catch it).

The "already-handled sound shapes" to skip (so the check is not over-strict) are exactly the
must-allow rows above: `matches!(lhs_type, Type::Null)` (already early-returns :1682),
`fallback_nullable` / `?? null`, the `widen_ints` and `checked_narrow` integer cases, and a
vector-literal default. For everything else the predicate is loft's **existing assignability
test** — `Parser::can_convert(&mut self, test_type, should)` (`src/parser/mod.rs:2747`), the
same one assignment/argument-passing consults: `!self.can_convert(&rhs_type, lhs_type.base())`
⇒ reject. (It exists already; no extraction needed. It takes `&mut self` — call it on the
resolved `rhs_type` before the coalesce is built.)

## Small safe steps

| # | Step | Proof |
|---|---|---|
| 0 | **Matrix → probes.** Graduate the table above to `/tmp` probes on `--interpret`, hand-computing each cell (SIGSEGV / garbage / valid value); confirm the harness can fail (a no-output cell is vacuous). Done — the tables are the matrix. | the boundary is exactly "assignable vs not", one-directional |
| 1 | **Confirm `can_convert` is the right predicate** (`mod.rs:2747`, already exists). Spot-check it returns true for the must-allow rows (same-type, `float?`←int, `u8?`←0) and false for the reject rows (`Row`←int, `int`←float, `int`←text) — a throwaway unit probe. No product change. | the predicate agrees with the matrix on both directions |
| 2 | **Add the gate, INERT (log-only).** At the chokepoint, when the default is not a sound special-case and `!self.can_convert(&rhs_type, lhs_type.base())`, `log`/`eprintln!` (env-gated) "would reject" instead of erroring. Run the WHOLE corpus + scripts + libs and inspect: it must flag ONLY the mismatch cases (the matrix reject rows) and NOTHING valid. | zero valid `??` flagged across `tests/scripts`, `tests/lib`, `default/*.loft`; the 5 reject cases flagged |
| 3 | **Flip to a hard error.** Replace the log with the `diagnostic!(Level::Error, code = "coalesce-default-type-mismatch", …)` reject + add the code to `tests/e1_code_set.rs`'s CODES. | the 5 reject programs now compile-error on BOTH backends (no SIGSEGV, no garbage, no E0308-only-on-native); every valid shape still compiles + runs |
| 4 | **Regression tests.** `parse_errors.rs`: one `code!(…).error(…)` per reject class (ref/scalar, int/float, cross-type). `tests/scripts/`: a script asserting the valid shapes (same-type, float?/int, `?? null`, `?? []`, checked-narrow) still compute right on both backends. Graduate the `h[absent]` case into the behaviour corpus now that it errors cleanly (or keep the type-correct `h[k].v` form). | both new tests green; the corpus can add the keyed-collection absent case |
| 5 | **Verify the full matrix on both backends + suite.** Re-run the matrix on `--interpret` AND `--native`; run `make ci`. | matrix all-reject-or-valid as designed; suite green |

## Falsification (how the fix could be wrong)

- **Over-reject** → a valid `??` newly errors (a real break, and pre-freeze an error is a
  one-way door). Mitigation: step 2's log-only pass over the ENTIRE corpus/scripts/libs is
  the falsifier — the gate must flag *nothing* valid before it becomes an error. The
  one-directional numeric case (`float? ?? int` valid, `int? ?? float` reject) is the sharp
  edge — assert both directions explicitly.
- **Under-reject** → a mismatch still slips through and crashes. Mitigation: the matrix's
  reject rows are the positive controls in step 3; add any third mismatch shape found in
  step 2 before flipping.
- **Wrong chokepoint** → fixing the *interpreter* to not crash (instead of rejecting at
  compile) would leave the silent-corruption cases (garbage values) unfixed and the backend
  divergence intact. The compile-time reject is the only fix that closes ALL rows at once
  and on both backends — enforce the invariant where it is violated (the type check), not at
  the crash site.

## See also
- `src/parser/operators.rs:1602` (`build_null_coalesce_default`) — the chokepoint.
- `Parser::convert` (cast/assignment coercion) — the assignability logic to reuse read-only.
- [flip-gate-coverage-gaps.md](flip-gate-coverage-gaps.md) — where the bug was found.
- [formal-audit.md](formal-audit.md) § error surface — the E2 "add every error" one-way door.
