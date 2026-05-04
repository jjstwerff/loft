<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# User-Facing Deferred Work

Items that downstream developers using loft would notice if shipped
— bugs they'd hit in normal code, features they'd want to use,
performance issues they'd benchmark.  Drawn from the validation
matrices and roadmap; cross-referenced with `DEFERRED.md` (the
internal-deferred index) and `ROADMAP.md` (the planned-feature
ladder).

**Criterion for inclusion:** *would I include this in a release
note?*  If yes — it belongs here.  Internal refactors,
test-infrastructure work, and quality items invisible to
downstream developers stay in `DEFERRED.md` only.

**Lock-in mechanism:** every item below has a `Lock-in test`
column.  The test is `#[ignore]`d today and goes green
automatically when the fix lands.  Pre-release ritual:

```bash
cargo test --release -- --ignored 2>&1 | grep -E "^test (plan-?[0-9]|P[0-9]+|T[0-9])"
```

Items the grep shows as **ok** are ready to ship; un-ignore them
and add a release-note entry.  Items still **FAILED** stay
deferred for the next release.

---

## Open user-facing items

| Item | What user hits today | Workaround | Surfaced by | Lock-in test |
|---|---|---|---|---|
| **Implicit generic-tuple type inference** | `t = min_max(7, 3); t.0` rejects with "Expect token ;" — the parser doesn't propagate the substituted return type to the receiving variable | Annotate the receiving slot: `t: (integer, integer) = min_max(...)` (then `t.0` works) | plan-17 phase 01 (A) caveat | `tests/issues.rs::plan17_a_implicit_generic_tuple_type_inference` |
| **Bounded-T method-call return inference** | `<T: Printable>(x: T) -> text { x.to_text() ++ "!" }` rejects with "No matching operator '+' on 'unknown(0)' and 'text'" — `x.to_text()` returns `Type::Unknown` instead of the bound's declared `text` | Bind to a typed local first: `s: text = x.to_text(); s ++ "!"` | plan-17 phase 01 (B) | `tests/issues.rs::plan17_b_bounded_method_return_type_propagates` |
| **`name @ pattern` inside or-patterns** | `match n { x @ 1 \| x @ 2 => x*10, _ => 0 }` rejects with "Expect token =>"; the parser doesn't recognise `name @ pattern` in the or-pattern loop.  No longer hangs (plan-18 phase 01 fix) but the syntax doesn't work | Either: (a) bind in the arm body — `1 \| 2 => { x = n; x*10 }` — or (b) split into separate arms | plan-18 phase 01 feature decision | `tests/parse_errors.rs::plan18_at_binding_in_or_pattern_does_not_hang` (regression-only; **add lock-in for the feature**) |
| **Native char-tuple-elem comparison** | `t.0 == 'a'` where `t: (character, _)` errors under `--native` with rustc E0308 (`expected i32, found char`) | Cast to integer: `t.0 as integer == 97`.  Or destructure first: `(c, _) = t; c == 'a'` | plan-14 phase 01 / P207 | `tests/tuple_matrix.rs::e1_d1_char_int_local` |
| **Tuple-returning par workers** | `for x in v par(r = make_tuple(x), 4) { ... }` rejects "Parallel worker return type 'tuple(...)' (size 16) is not supported" | Wrap result in a single-field struct (`struct Pair { v: (A, B) }`) and have the worker return `Pair`, then `r.v.0` / `r.v.1` | plan-06 phase 9c | `tests/threading_chars.rs::par_tuple_return_int_int`, `_int_text`, `_struct_text`, `_three_arity`, `_nested` (5 ignored canaries) |
| **Tuple destructure in fused-for-par** | `for (a, b) in pairs par(r = work(a, b), 4) { ... }` rejects "Expect variable after for" | Pre-bind: `for p in pairs par(r = work(p.0, p.1), 4) { ... }` | plan-06 phase 9d | `tests/threading_chars.rs::par_tuple_destructure_in_for` |
| **Capturing closures in `vector<fn(...)>`** | `vector<fn(integer) -> integer>` of heterogeneous capturing lambdas fails at vector-construction with type-inference or store-write panic.  Non-capturing lambdas and named fn-refs work fine | Restrict to non-capturing lambdas, named fn-refs, or homogeneous closure types | plan-06 phase 1 retrospective + plan-15 D4 | `tests/threading_chars.rs::par_vec_of_capturing_fns_t4` |
| **Closure-DbRef leak** | A capturing closure stored in a struct field or local variable is **not freed** at scope exit.  The 16-byte fn-ref slot's closure DbRef is currently leaked.  No user-visible symptom in short-lived programs but accumulates in long-running ones (servers, REPLs, repeated benchmarks) | None today — closure usage is bounded in practice; bigger concern for upcoming server library | plan-15 phase 03 (active risk per LIFETIME.md "NOT YET HANDLED") | none yet — to be added during plan-15 phase 03 |

---

## Recently shipped (released-not-yet)

Items closed in the current branch (`plan-14-tuple-validation`) that
will appear in the next release notes:

| Item | Closed by | Test |
|---|---|---|
| Match-arm separator must be `=>`, not `->` (parser hung on `->` before fix) | P206 / plan-14 phase 01 | `tests/parse_errors.rs::p206_match_arrow_rejected_*` |
| Tuple-of-text return under `--native` (`fn f() -> (text, text)` produced rustc E0308 `Str` vs `&str` mismatch) | T1.8a / plan-17 phase 01 | `tests/tuple_matrix.rs::e2_d2_return_text_text` |
| Bounded generic returning tuple `<T: Bound>(...) -> (T, T)` (return type stayed parametric `(DbRef, DbRef)` while params substituted to `i64`) | plan-17 phase 01 (A) | `tests/issues.rs::plan17_generic_tuple_return_with_annotation` |
| Built-in types satisfy `Printable` automatically | plan-17 phase 01 (C) | `tests/issues.rs::plan17_printable_integer_satisfies` |
| Or-pattern with `@`-binding no longer hangs the parser (now reports clear error and recovers) | plan-18 phase 01 | `tests/parse_errors.rs::plan18_at_binding_in_or_pattern_does_not_hang` |

---

## Closed-by-decision (will not ship)

Items declined as design non-goals.  Recorded here so a future
contributor finds the decision before re-proposing.

| Item | Decision | Rationale | Reference |
|---|---|---|---|
| `iterator<T>` as par input or output | Non-goal | Generators are sequential; par needs random-access input. Worker-state-handoff has no real consumer | DESIGN.md D11c.2 (plan-06) |
| Tuples in struct fields (T1.11a) | Originally rejected, **lifted in 0.8.4** — tuple fields work via inline layout (synthetic `__tuple<…>` struct) | per `tests/parse_errors.rs` line 765 comment | TUPLES.md / `parser/mod.rs::set_field_check`/`get_val` Type::Tuple arms |
| Single-element tuples `(T,)` | Non-goal — `(T)` is just `T` | TUPLES.md |
| Named tuple fields `(name: T, value: U)` | Non-goal — use a named struct | TUPLES.md |
| Whole-tuple format-string interpolation `{t}` | Non-goal — access elements explicitly | TUPLES.md |
| Vector-of-capturing-closure with **heterogeneous** captured shapes | Restriction documented per loft-write skill; lift requires fundamental vector-storage rework | DESIGN.md D11a row 8 (split) |
| Dynamic dispatch / interface values (`x: Ordered = …`) | Non-goal — interfaces are constraints, not types | INTERFACES.md |
| Composite interfaces / interface inheritance | Non-goal | INTERFACES.md |
| Default method implementations on interfaces | Non-goal | INTERFACES.md |
| Associated types | Non-goal | INTERFACES.md |

---

## How items move through this file

```
USER_FACING.md (open)  ──fix lands──►  USER_FACING.md (recently shipped)
                                                │
                                                ▼
                                 release notes + CHANGELOG.md
                                                │
                                                ▼
                              row removed from USER_FACING.md
```

Closed-by-decision items stay forever as historical record.  When a
new "would-this-affect-users?" question is raised, grep this file
first for the answer.

---

## Severity override — when to break "finish plans first"

The default discipline is **finish the validation plans before
shipping new feature work** (see plans/README.md preamble + the
"finish-plans-first" stance recorded in this branch's session
history).  Validation work compounds; splitting attention across
plans + features is slower than serialising.

**But:** an item in the open table above can override that
discipline if its severity is high enough.  If a user-facing item
is **too aggrieving to ship a release with**, fix it before
release regardless of which plan it belongs to and regardless of
how many plan phases are still in-flight.

### Severity thresholds — `Severity` column

Each open row carries an implicit severity tier.  Make the call
explicit when uncertain.  The tiers, from must-fix-now to
defer-acceptable:

| Tier | What it means | Examples from the current table |
|---|---|---|
| **S0 — release-blocking** | User code that follows the documented language reference fails or crashes.  Visible in basic patterns; no reasonable workaround.  Example: a runtime panic on `sorted<>` cleanup that we can reproduce. | (none today; plan-20's panic was here but doesn't reproduce) |
| **S1 — ship-with-caveat-OK once** | Visible to users but with a clear, documented workaround that idiomatic loft code uses anyway.  Acceptable for one release with a release-note caveat; **must be fixed before next release**. | "Implicit generic-tuple type inference" (workaround: explicit annotation); "Bounded-T method-call return inference" (workaround: typed local) |
| **S2 — niche / advanced-pattern** | Affects an idiomatic shape but not the language's primary surface.  Workaround is straightforward; users can avoid the path entirely. | "`name @ pattern` inside or-patterns"; "Tuple-returning par workers"; "Capturing closures in `vector<fn(...)>`" |
| **S3 — cross-cutting future-feature** | Fix is part of a planned feature that hasn't been scheduled yet.  Documenting the gap is the deliverable. | "Closure-DbRef leak" (planned plan-15 phase 03); coroutines × par non-goal |

### Override decision rule

If the open table contains:
- **any S0 row** → block release until fixed.
- **an S1 row that's been deferred for two releases in a row** → promote to "must-fix-before-release" status; if it can't be fixed in time, slip the release.
- **only S2/S3 rows** → ship; document workarounds in release notes.

When promoting an S1 → must-fix:
1. Write a `Trigger to unpause:` line on the deferring plan if not already present.
2. Move the work to the *front* of the next session's queue.
3. Add the override note to the release-prep checklist.

This rule keeps the "finish plans first" discipline as the
*default* while preserving the right to override when severity
demands it.  The bar for override is "user code can't do its job"
(S0) or "we keep deferring this and it's been long enough" (S1
with two-release decay).  Pure feature gaps (S2/S3) wait their
turn in the matrix.

### Today's snapshot (2026-05-04)

All 8 open rows in the table above are **S1** or **S2**:
- "Implicit generic-tuple inference" — S1 (would block 1.0 if
  still unfixed by then; documented workaround exists for now).
- "Bounded-T method-call return inference" — S1 (same shape).
- "`name @ pattern` in or-pattern" — S2 (alternate syntax exists).
- "Native char-tuple-elem comparison" — S2 (cast workaround).
- "Tuple-returning par workers" — S2 (struct-wrap workaround).
- "Tuple destructure in fused-for-par" — S2 (pre-bind workaround).
- "Capturing closures in `vector<fn(...)>`" — S2 (named-fn-ref
  workaround).
- "Closure-DbRef leak" — S3 (no current symptom; long-running
  programs only).

**No S0 today.**  Plan-20's panic would have been S0 if it
reproduced; it doesn't, so it's deferred with a re-surface
trigger.  Continue with the "finish plans first" default.

---

## Cross-references

- [DEFERRED.md](plans/DEFERRED.md) — internal-deferred work index
  (every parked item with its trigger to resume).
- [ROADMAP.md](ROADMAP.md) — planned-feature ladder by milestone
  (0.9.0 / 1.0.0 / 1.1+).
- [PROBLEMS.md](PROBLEMS.md) — open P-issues; closed entries removed
  per file convention.
- [DESIGN_DECISIONS.md](DESIGN_DECISIONS.md) — long-form rationale
  for closed-by-decision items above.
- [CHANGELOG.md](../../CHANGELOG.md) — user-facing release notes.
