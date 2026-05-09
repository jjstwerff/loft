<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Closed-by-Decision Register

Items evaluated for inclusion and **explicitly declined** after
review.  This file exists so the same questions don't resurface in
every session.  Before proposing one of these as a bite, plan item,
or PR, check the entry here — if the situation hasn't materially
changed, the decision stands.

**Workflow rule** (see [DEVELOPMENT.md § Using this
register](DEVELOPMENT.md#closed-by-decision-register)):

- Closed-by-decision items are not "backlog" and should not appear
  in ROADMAP.md's scheduled milestones, PLANNING.md's priorities,
  or QUALITY.md's active tables.  A short reference in the
  "Out of scope" sections of those docs is sufficient.
- Re-opening requires **new evidence**: a concrete use case,
  incident report, or performance measurement that wasn't available
  at the decision.  Bring that evidence to the top of the revived
  entry; don't silently flip the decision.
- Adding a new entry requires the same rigor: the question, the
  evaluation, the decision, and the conditions under which the
  decision would change.

---

## Format

Each entry has:

1. **Question** — the proposal as it was raised.
2. **Evaluation** — the trade-offs weighed.
3. **Decision** — closed / accepted / partial, with the date.
4. **Revisit when** — the concrete trigger that would warrant
   reconsideration.

---

## C3 — WASM `par()` runs sequentially

**Question.** Should browser WASM builds parallelise `par()` loops
across a Web Worker pool?

**Evaluation.** Web Workers in a loft-compiled WASM require:
- Bundle-size overhead for the pool shim (KB per worker at minimum).
- Startup latency (cold-starting a worker is ~50 ms, dominates short
  frame budgets).
- Shared-memory configuration (`SharedArrayBuffer` requires COOP/
  COEP HTTP headers, which most loft hosting targets — itch.io,
  plain GitHub Pages — don't set).

None of the shipping loft programs are CPU-bound on the browser.
Brick Buster (the headline game) runs at 60 fps on a single thread.

**Decision.** **Closed — accepted limitation.**  Native target
keeps `par()` parallelism; browser is sequential.  Dated 2026-04.

**Revisit when.** A concrete loft program demonstrates a CPU
bottleneck on browser that can't be solved by algorithmic work, AND
the target host supports COOP/COEP headers.  Bring the profiler
trace.

---

## C38 — Closure capture is copy-at-definition

**Question.** Should lambda captures be by reference (like Rust
borrows) instead of by value (like Rust `move`)?

**Evaluation.** Reference capture requires either:
- **Garbage collection** — fundamental departure from loft's
  store-based heap with explicit scopes; changes every allocation
  path.
- **Borrow tracking** — Rust-style lifetimes and borrow checker;
  crosscuts every function signature, every type declaration, and
  the `#rust"..."` FFI layer.

Neither fits the "simple, fast, no lifetime annotations" language
ethos.  The value-semantic capture is also the less-surprising
default for beginners — the captured value is exactly what was
visible at lambda definition time.

**Decision.** **Closed — accepted design choice.**  Dated 2026-04.
Regression guard: `tests/scripts/56-closures.loft::test_capture_timing`.

**Revisit when.** A critical loft program cannot be expressed
ergonomically with value capture AND the alternative has been
prototyped to show it doesn't destabilise the store-based heap.

**Future direction (recorded 2026-05-04, not a re-opening).**  The
long-term ambition is to move closer to Rust's closure model —
borrow-checked `&T` / `&mut T`, FnOnce / FnMut / Fn capability
hierarchy, statically-enforced single-mutator-or-multiple-readers.
The current copy-at-definition model with `Reference<T>` and a
planned `Mutable<T>` stdlib helper covers the
[EventLoop](plans/future/23-event-loop/README.md) and first-game use cases acceptably;
the closure-model evolution should be designed against
real-world friction observed once a real game ships, not
pre-emptively.  Sequencing for the evolution lives in
[plans/future/22-mutable-closures/README.md](plans/future/22-mutable-closures/README.md) (the design spec) and
[plans/future/22-mutable-closures/DISCUSSION.md](plans/future/22-mutable-closures/DISCUSSION.md)
(alternatives considered, including the full Rust borrow-checker
option F).

---

## C54.D — Rust-style numeric literal suffixes

**Question.** Should loft accept `34u8`, `4948u32`, `100i32` as
literal syntax for explicit-width integer constants?

**Evaluation.** Loft's context-driven type inference already
handles every common case:

- `x: u8 = 255;` — range-check at the binding site.
- `f(a: u8)` called as `f(34)` — literal constrained by parameter
  type.
- Ambiguous cases — `34 as u8` (one existing operator, no new
  syntax).

Adding suffix syntax would:
- Crosscut the lexer (ambiguity with identifiers: `1u8` vs `1_u8`).
- Conflict with loft's "prefer the type annotation over the literal
  annotation" ethos (the binding site documents intent, not the
  literal).
- Solve a 1 % problem that `as` already covers.

**Decision.** **Closed — declined.**  Dated 2026-04-13.  See
[QUALITY.md § C54](QUALITY.md#active-design--c54-integer-i64) — `C54.D` listed under sub-tickets.

**Revisit when.** A real loft program needs a literal-size
distinction that cannot be expressed as `as <T>` in reasonable
syntax.  "I wrote it in Rust that way" is not sufficient evidence.

---

## C62 — No type annotations in `|x|` shorthand lambdas

**Question.** Should loft accept type annotations on shorthand
`|x|` lambda parameters (e.g. `|x: integer, y: integer| { x + y }`)?

**Evaluation.** Loft already has two orthogonal function syntaxes:

- `|x| { body }` — the **inferred** shorthand, designed for use
  inside `map` / `filter` / `reduce` and other higher-order calls
  where the expected parameter types flow in from the call site's
  lambda hint.  Its whole reason to exist is visual compactness.
- `fn(x: T, y: T) -> R { body }` — the **explicit** form, with
  full type annotations and an optional return type (omit `->` for
  void returns).  Use this when the types can't be inferred — for
  example, when the lambda is stored in a local variable before it
  reaches a call site.

Adding types to the shorthand:
- Collapses the distinction — the two forms now mean exactly the
  same thing (one with `|` delimiters, one with `fn(` keyword) and
  each style becomes a coin-flip.
- Blurs the "where types flow from" mental model: users stop
  asking "is this inferrable?" and start writing `|x: T|` by
  habit, defeating the point.
- Complicates the parser (currently `|x|` is unambiguous with
  `|` as bitwise-or via lookahead on parameter shape; adding `: T`
  introduces more disambiguation branches).

Users who want types should use `fn(...)`.  There is no scenario
where `|x: T| { ... }` is the only viable syntax — if types are
wanted, `fn(x: T) { ... }` has every capability plus an explicit
return type when needed.

**Decision.** **Closed — declined.**  Dated 2026-04-17.  The
compiler rejects `|x: T| { ... }` with an error that points at
the `fn(x: <type>) { ... }` form (P169 updated the wording).

**Revisit when.** Never, barring a language-level change that
eliminates the inferred-shorthand / explicit-fn distinction
altogether (i.e. a fundamental rewrite of the lambda story).

---

## C63 — No nested `fn` definitions inside fn bodies

**Question.** Should loft accept `fn` declarations inside another
function's body (e.g. `fn outer() { fn helper(x: integer) -> integer
{ x * 2 } helper(5) }`)?

**Evaluation.** Loft already has two orthogonal forms for the
"function-shaped local helper" use case:

- `let helper = |x| { x * 2 };` — closure shorthand, inferred types.
- `let helper = fn(x: integer) -> integer { x * 2 };` — typed
  closure form.

Both bind a function value to a local variable, callable inside
the parent fn (`helper(5)`), invisible outside, dropped at scope
exit.  Adding a third syntax via nested `fn` declarations forks
into one of two semantic choices, neither of which is good:

1. **Captures parent locals** — re-implements closures with
   different syntax.  Two forms now mean the same thing; users
   coin-flip between them with no clear heuristic.  Documentation
   surface grows for cosmetic-only gain.
2. **Doesn't capture** — contradicts every mainstream language
   (Python, JS, Rust, Swift all give nested fns access to outer
   locals).  Users write `fn helper() { use(outer_var) }` expecting
   it to work; the parser rejects with "can't find outer_var."
   That's a worse experience than the current "no nested fns"
   rule, which fails clearly with a single-line diagnostic.

Implementation cost is also non-trivial: parser path for local
fn decls; scope analysis for capture/no-capture; codegen path or
desugaring rules to existing closures; edge cases for forward
refs, recursive nested fns, mutual recursion.  Estimated 1-2
focused sessions for a feature whose runtime benefit is zero
(closures already cover every use case) and whose only gain is
cosmetic locality.

The "with working closures" framing is the giveaway: if inline
fn semantics ARE just "closure with implicit name binding," the
feature is pure sugar.  Sugar that's not load-bearing for any
concrete user need is exactly what pre-1.0 should refuse —
every additional surface is one more thing to validate,
document, and maintain.

**Decision.** **Closed — declined.**  Dated 2026-05-04.  The
parser continues to reject `fn` definitions inside function
bodies with the existing `'fn' definitions must be at file
scope, not inside a function or block` diagnostic.  The
loft-write skill's "Nested fn definitions are forbidden"
section documents the workaround (typed lambda).

**Revisit when.** A concrete user workflow surfaces where the
typed-lambda form (`let helper = fn(x: T) -> R { ... };`) is
genuinely awkward enough to cost more developer time than the
inline-fn implementation cost.  Today (2026-05-04): no such
workflow has surfaced; the loft-test cross_mode harness pulls
helper fns to file scope cleanly, and `lib/graphics/` plus the
stdlib never need local-helper recursion.

---

## Adding a new entry

When closing a question, append a new `##` section using the
format above.  Follow with a one-line pointer from the source
document's "Out of scope" table:

```markdown
| CXX | Title | Closed — see [DESIGN_DECISIONS.md § CXX](DESIGN_DECISIONS.md#cxx) |
```

Do not move the question itself out of the source doc's history.
Strike it (`~~…~~`) and point at this register.  That keeps the
original context discoverable from git blame / git log without
cluttering active tables.
