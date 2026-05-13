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

**Plan-22 addendum (shipped 2026-05-13).**  Plan-22 (mutable
closures) ships implicit-by-body mutation classification on top
of C38.  The "copy-at-definition" framing now applies only to
truly-immutable scalar captures in pure read-only contexts:

- Captures of `Type::Reference` (struct, nested struct) always
  use 12B `Parts::DbRef` pointing at the live original (P260 fix,
  `src/parser/vectors.rs::synthesize_closure_record`).  Mutations
  from either side are visible immediately.
- Captures of scalars whose bodies write to the capture are
  promoted to heap-owned cells via the phase-02d-iii.a type flip
  (`Type::Reference(__cell_<T>, vec![])` encoding).  The outer
  scope and all closures share the same cell.
- Pure read-only scalar captures remain value-copy (Case A
  semantics — unchanged).

Case D ("aliased mutating") was decommissioned 2026-05-13: the
cell + auto-Reference machinery from phases 02-03 already gives
shared-state semantics, so no rejection was needed.  See
[plans/finished/22-mutable-closures/04-case-d.md](plans/finished/22-mutable-closures/04-case-d.md)
for the major finding.  Design history and alternatives
considered: [plans/finished/22-mutable-closures/DISCUSSION.md](plans/finished/22-mutable-closures/DISCUSSION.md).

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

## C64 — Tuple struct-ref elements use MOVE semantics (not copy + null)

**Question.** When a tuple element is a struct reference
(`Type::Reference`), what semantics apply at scope exit and on
destructuring?  Two candidates:

- **Move semantics:** the source tuple's scope-exit emits no
  per-element `OpFreeRef`; each destructured destination variable
  owns its element.
- **Copy + null semantics:** the destructure copies the DbRef,
  then nulls out the source tuple's slot via a new
  `OpNullTupleElem(var, offset)` opcode so the source's scope-exit
  cleanup is a no-op for the moved-out element.

**Evaluation.** Move semantics is what the runtime already does:
`src/scopes.rs:1000-1009`'s tuple scope-exit arm is a `continue`
stub (no per-element `OpFreeRef` on the source tuple), and
`parser/expressions.rs::parse_assign`'s destructure path types
each destination variable as the source element's `Type::Reference`
via `change_var_type(v_nr, &rhs_elems[i])` so the destination
gets ordinary scope-exit cleanup.  The Plan-14 phase 04 cross-mode
harness exercised every single-iteration E5 shape (swap, arg,
return, mixed Ref+int, mixed Ref+text, plain local) on both
backends with byte-identical output and no panics — confirming
move semantics is correct in practice.

The "copy + null" alternative would require a new opcode at a
time when the opcode space is near-saturation (254/256 used per
CHANGELOG_TECHNICAL.md), would add a per-destructure runtime
write the move path doesn't need, and produces no observable
difference to user code — only the runtime cleanup ordering
changes.  No concrete program shape exists that move-semantics
gets wrong but copy+null would get right.

The loop-iteration aliasing bug (P250 — `for { (q1, q2) =
make_pair(pa, pb); }` reads `null` for whichever destructured
variable picked up the FIRST argument on iterations >0) is a
SEPARATE dep-tracking issue, not a move-vs-copy semantics
question.  Both candidates would have the same loop-iter problem
without an additional fix; the bug lives in the destructure
path's dep propagation between source argument slot and
destination variable slot.

**Decision.** **Move semantics.**  Dated 2026-05-11.  The
runtime path is locked by 6 cross-mode E5 cells in
`tests/tuple_matrix.rs` (e5_d1_struct_ref_local + swap +
ref_int_local + ref_text_local; e5_d2_struct_ref_arg + return).
Plan-14 phase 04 records the rationale.

**Revisit when.** A concrete shape appears where move semantics
is observably wrong but copy + null would be correct — none
known as of 2026-05-11.  P250's fix lives in dep-tracking, not
in the move/copy axis.

---

## C65 — Tuple "structure value" element type folded into reference (E5 = E6)

**Question.** Should the validation matrix carry a separate E6
element type for "structure value" (an inline by-value struct
copied into the tuple slot, distinct from `Type::Reference`)?

**Evaluation.** Loft has no inline by-value struct type distinct
from `Type::Reference`.  A `struct Foo { ... }` declaration
produces a record laid out in a store; the loft-level "value" you
pass around is a `Reference(struct_def, dep)` — a 12-byte
`DbRef`.  Tuple element E5 (Reference) already covers this shape
end-to-end.

Carving out a separate E6 row would either (a) duplicate every
E5 cell with no semantic difference, or (b) require introducing
a new `Type::StructValue` variant — a substantial language-design
change with no consumer.  Neither pays for itself.

**Decision.** **Folded into E5.**  Dated 2026-05-11.  Plan-14's
matrix in `00-matrix.md` marks every E6 cell as
`CLOSED:folded into E5`.  A future feature that introduces inline
value structs (none on the roadmap) would re-open the row.

---

## C66 — Production loft programs never abort on user-attributable edge cases (development may halt)

**Question.** Should runtime fault sites (divide-by-zero, vector /
text out-of-bounds, null DbRef dereference, narrow-cast overflow,
`panic("msg")` / failed `assert`) HALT the loft program with a typed
runtime error (the rustc-style "loud failure" model)?  Or should they
return a silent sentinel (null / `i64::MIN` / null DbRef / char 0)
and let execution continue?

**Evaluation.** Loft's primary deployment target is **interactive
programs that must not stop**: browser games shared via URL, native
games with continuous frame loops, scriptable scenes inside
applications, multiplayer servers driving live sessions.  In every
one of those contexts an abort is far worse than a wrong-pixel /
wrong-frame / wrong-record edge case.  A frozen game cannot be saved
by the user; a wrong pixel can.

But during **development** — running tests, debugging a script,
iterating on a feature — halting on a runtime fault is a feature,
not a bug.  Halt-on-fault is how you find the divide-by-zero you
didn't know about, the OOB the test missed, the assert that's
firing.  The dev tooling (CLI, test harness, REPL) wants the loud
failure mode; the production game / server / browser embed does
not.

This separation is not new.  Loft has always carried it via the
`production` flag on the logger:

- `default/01_code.loft::panic / assert` are documented as
  **logging in production** (`#impure(host_io)`); the
  production-mode path in `src/native.rs::n_panic` and `n_assert`
  checks `logger.config.production`.  When `production == true`
  it writes a fatal log entry, sets `Stores::had_fatal = true`, and
  **returns** — execution continues.  When `production == false`
  (or no logger attached) the same site halts via the typed
  `RuntimeError` path so the developer sees the failure.
- `main.rs` reads `had_fatal` after execute returns and exits 1
  ONLY when no frame loop is running.
- Integer `/` by zero, `%` by zero, vector / text OOB, null DbRef
  field reads, narrow-cast overflow today return null sentinels
  (`i64::MIN`, `char(0)`, `DbRef { rec: 0 }`) and let downstream
  code keep running — even in dev mode.  Plan-07 phase 4
  converts these to typed-error halt **in dev mode only**;
  production keeps the silent + log shape.
- The `??` operator is the user's tool for explicit handling of
  null sentinels: `x = a / b ?? 0` discharges the null with a
  fallback at the user's choice.
- The `RuntimeLogger` framework (`doc/claude/LOGGER.md`) is the
  surface for surfacing edge-case incidents to operators without
  halting the program.

**Decision.** **Loft programs in production MUST NOT abort on
user-attributable runtime edge cases.**  Dated 2026-05-11.
Development is unaffected; halt-on-fault is the right behaviour
for the test runner, the CLI, and interactive debugging.  The
gate is the existing `Stores::logger.config.production` flag.

For each fault site:

| Site | Production (`logger.config.production == true`) | Development (default) |
|---|---|---|
| Integer `/`, `%` by zero | log warning, return `i64::MIN` sentinel, continue | typed `RuntimeError::DivideByZero`, halt + render |
| Vector / text OOB (positive, negative past start) | log warning, return null DbRef / char 0, continue | typed `RuntimeError::IndexOutOfBounds` / `NegativeIndex`, halt + render |
| Null DbRef field / method access | log warning, return null result, continue | typed `RuntimeError::NullDereference`, halt + render |
| Narrow-cast overflow | log warning, return clamped or null, continue | typed `RuntimeError::NarrowCastOverflow`, halt + render |
| `panic("msg")` builtin | log fatal, set `had_fatal`, return | typed `RuntimeError::UserPanic`, halt + render |
| Failed `assert(test, msg)` | log error, set `had_fatal`, return | typed `RuntimeError::AssertionFailed`, halt + render |
| Stack-overflow trap | log fatal, attempt graceful unwind | typed `RuntimeError::StackOverflow`, halt + render |

**Three-way defense contract** — codegen picks the opcode based on
how the user's code handles the fault potential:

| Defense at compile-time | Emit | Runtime behaviour |
|---|---|---|
| `expr ?? fallback` | Nullable peer (existing `parser/operators.rs::rewrite_outer_arith_to_nullable` extended to vector/text) | No log, no halt; sentinel discharged by `??` |
| `if x != null { use(x); }` (or `if !x { … }`) immediately after `x = expr` (statically detectable defensive check) | Nullable peer (new flow-analysis arm in parse_assign) | No log, no halt; user's defense handles the null |
| Neither | Raising peer | Production: log warning + continue with sentinel.  Development: halt + render. |

The static-analysis fallback "raising peer + runtime warning" is
the safety net — it catches sites the developer forgot to defend
AND sites where the defense lives across a function boundary
(can't be statically detected).  In production those sites still
produce a sentinel and execution continues; the log entry surfaces
the issue to operators for investigation.

A compile-time warning fires at every undefended fault site that
the parser can statically recognise as fault-prone:

```
warning: `v[i]` may produce null on out-of-bounds with no defensive check
  --> game.loft:42:8
   = note: guard with `if i < len(v) { ... }` before indexing
   = note: or accept null with `v[i] ?? <fallback>`
   = note: or follow with `if x != null { ... }` to catch the null
```

Silenceable via `LOFT_NO_WARN_RUNTIME=1` for codebases where the
warning rate is too high (rarely needed once defensive idioms are
adopted).  The warning is BOTH a quality nudge for developers AND
a way to silence the production-mode runtime log: defending the
site (any of the three patterns) makes the warning go away AND
the log goes silent.

**Production mode REQUIRES a logger.**  A deployment configured
for production with no logger attached MUST refuse to start with a
clear, actionable error message — there is no "production but no
logging" middle state.  The reason: in production every runtime
event needs a destination, and silently swallowing them defeats
the entire point of choosing the production path.  The startup
check fires before user code runs:

```
Error: production mode requires a logger.
Attach one via `loft::logger::Logger::attach(...)` at startup,
or configure via the deployment's logger settings (see
doc/claude/LOGGER.md § Production setup).  To run in development
mode (halt-on-fault), unset the production flag.
```

The deployment is expected to attach a real sink (stderr, file,
syslog, host bridge, etc.) at startup.  If a deployment genuinely
wants "log + continue but discard the output", it attaches a
no-op sink — that's a deliberate choice, made visible in the
deployment config, not an accident from forgetting to attach a
logger.

**Implementation shape (Plan-07 phase 4 reframe 2026-05-11):**

The typed-error infrastructure (`RuntimeError` type +
`RuntimeErrorKind` variants + `--> file:line:col` + caret rendering
through phase-2's `render_entry_pretty`) is **kept** — it's the
right shape for both the dev-mode halt diagnostic and the
production-mode log entry.  Per-site helpers (`State::raise`,
`vec_get_or_raise`, `vec_ref_or_raise`, `text_char_or_raise`)
need a production-mode branch added:

```rust
// In State::raise (sketch):
let production = self.database.logger.as_ref()
    .and_then(|l| l.lock().ok())
    .is_some_and(|l| l.config.production);
if production {
    // Production: log + had_fatal, do NOT short-circuit code_pos.
    // The op returns its sentinel; execution continues.
    // The logger is GUARANTEED present here — startup refuses
    // production mode without one.
    if let Some(logger) = &self.database.logger {
        if let Ok(mut lg) = logger.lock() {
            lg.log_runtime_kind(&kind, position.as_ref());
        }
    }
    self.database.had_fatal = true;
    return;
}
// Development: keep the typed-error halt path that's already in place.
self.database.runtime_error = Some(Box::new(RuntimeError { ... }));
self.database.had_fatal = true;
// (dispatch loop sees runtime_error.is_some() and short-circuits)
```

The `OpGetVectorNullable` / `OpVectorRefNullable` /
`OpTextCharacterNullable` opcodes added 2026-05-11 stay as the
form for **loop iteration end** specifically — they return null
without logging in either mode (end-of-iteration is expected
behaviour, not a fault).  The raising peers (`OpGetVector`,
`OpVectorRef`, `OpTextCharacter`) log + return null in production
and halt + render in development for the user-facing `v[i]` /
`s[i]` paths.

**Revisit when.** A concrete deployment shape surfaces where the
production-mode silent + log path is wrong.  No such case is
expected; the existing `n_panic` / `n_assert` production-mode
behaviour has held for years without complaint.

### Workflow corollaries (added 2026-05-11)

The 2026-05-11 evaluation of the C66 framework against the
day-to-day loft-development workflow surfaced four corollaries
that bind the abstract C66 rule to the developer experience.
All four are tracked under plan-07 phase 4:

1. **Format strings are observability, never raise** — every
   `{...}` interpolation auto-swaps to its Nullable peer at
   parse time.  The `println("{x}")` you reach for to inspect
   a bug must NEVER itself become the next bug.  Shipped as
   phase 4e.1 (commit 8e74aa16).  Reasoning: a halt or log
   inside the developer's diagnostic surface defeats the
   point of the diagnostic.

2. **Easy-proof skip list is REQUIRED for the warning** — the
   4e.2 compile-time warning at undefended fault sites must
   recognise the four canonical safe patterns (bound loop
   variable / explicit length check / constant-literal
   divisor / constant-literal index against known-length
   vector) BEFORE landing.  A noisy warning gets disabled
   within a session and the safety net evaporates; the skip
   list is a release blocker, not a follow-up.  See
   `plans/07-error-messages/04-runtime-error-kinds.md
   § Easy-proof skip list — REQUIRED for 4e.2`.

3. **State snapshot at fault site is the next-highest-leverage
   workflow win** — the dev-mode halt today says *where* the
   fault was but not *what* the values were.  Phase 4g.2
   captures the named-arg values + indexed-collection length
   into the rendered diagnostic so the developer sees
   `damage = [10, 20, 30] (len=3), idx = 5` without having
   to add a print statement to discover it.  The values are
   already on the bytecode stack at fault time; surface them.

4. **`not null` field reminder closes the long-term failure
   mode** — when a struct field is read 47× across the
   codebase and never compared to null, marking it `not null`
   at the constructor eliminates the entire class of fault
   sites for that field.  Phase 4h emits a `Level::Hint`
   pointing at the constructor when the read pattern says
   "this is morally not-null, mark it so."  Strictly better
   than defending each read site individually with `?? null`.

The corollaries together prevent the failure mode where
`?? null` becomes pervasive defensive boilerplate that the
2026-05-11 evaluation flagged ("everyone writes `?? null`
everywhere because the warning fired").  4e.1 + 4h reduce
the *count* of sites where the warning could fire; 4e.2's
skip list ensures the warning only fires where defending
is actually needed; 4g.2 makes diagnosis fast when defence
is needed and not yet in place.

---

## C67 — Fail at startup, not at runtime (no programmer-side try/catch for internal bugs)

### Question

When a loft program hits an internal-bug runtime panic (a
`todo!()`-stubbed native, a codegen mistake, an unimplemented
operator), should we add programmer-side error-recovery
constructs (`try { … } catch err { … }`, `#panic_safe fn(T)`
annotations, defensive `unwrap_or_else` boilerplate) so the
program can continue?

### Evaluation

Three competing pressures:

1. **Robustness perception** — users want loft programs to
   "just work."  A panic mid-execution feels broken regardless
   of whose fault it is.
2. **Programmer burden** — every defensive-wrap mechanism
   forces the user to remember a pattern.  Forgetting one
   wrap means a production crash.  The pattern proliferates
   ("everyone writes try/catch everywhere") until it's
   indistinguishable from the language not having had error
   recovery at all.
3. **VM-route fail-fast** — production loft programs run
   under a supervisor (systemd, kubernetes, containerd) that
   already monitors process exits and decides restart /
   escalate / page.  Hiding crashes from the supervisor
   defeats its job.

Three layers of failure exist:

- **Internal bugs** (todo!() stubs, type mismatches, codegen
  errors) — the LANGUAGE / RUNTIME made the mistake.  Should
  never reach a running program.
- **Startup-time external faults** (config invalid, port bind
  failure, missing deps) — the PROGRAM can't sensibly continue.
  The supervisor needs to know.
- **Steady-state external faults** (network drop, disk full,
  transient I/O error) — the LIBRARY handles them gracefully
  (auto-reconnect, retry, fallback to default).  The user-code
  in the loft program never sees them.

### Decision

**Closed (2026-05-13)** as a layered policy.  The user's framing:

> *"I do not want to burden the programmer with… write try
> catch or your program will fail.  So everything will need a
> try catch to function, we should not go that path.  Things
> have to function properly.  We can fail but that should be
> on startup and not during the running of a program."*
>
> *"We can still allow for runtime exceptions in the future,
> things can and will break.  Though the detection of it needs
> to be in a start-up phase, we go the VM route of failing
> fast so the VM manager is informed of a problem."*
>
> *"After initial startup we will do our utmost best to keep
> running."*

Three load-bearing rules:

1. **Internal bugs are caught at compile time, not at
   runtime.**  The codegen refuses to emit a binary that
   contains a reachable `todo!()` stub or any other
   would-panic-on-call construct.  If a loft program would
   panic from an internal mistake when called, the build
   fails with a clear message ("native fn `n_X` has no
   implementation; wire it in src/codegen_runtime.rs or run
   via --interpret").  This is the **compile-time** layer.
   Sibling work: every codegen path that historically emitted
   `todo!()` for an unimplemented native is now a hard
   compile-error per P269.
2. **Startup faults exit the program with non-zero status.**
   No catch_unwind, no logging-and-continuing.  The supervisor
   sees the exit code and decides.  This is the **VM-route
   fail-fast** layer.
3. **Steady-state runtime faults are HANDLED BY LIBRARIES,
   not by user code.**  lib/web's WebSocket auto-reconnects.
   lib/server's eventual `serve(handler)` primitive owns its
   own per-request `catch_unwind` (logs + 500 + continues
   serving).  lib/io returns explicit error values rather than
   panicking.  The USER's loft program never writes
   `try { … } catch { … }` for these — the LIBRARY hides the
   mechanism behind a clean API.  This is the **best-effort
   keep-running** layer.

**No `try { … } catch err { … }` language construct, no
`#panic_safe` annotation, no programmer-side `?? null`
boilerplate** for internal-bug recovery.  Loft's typed-error
infrastructure (per CLAUDE.md) IS allowed — that's about
EXPLICIT user-domain errors (parse failures, validation,
business-logic invariants) where the user wrote code that
returns `Result`-like values and consciously handles them.
That's different from internal-bug recovery, which is what
this decision rejects.

### Revisit when

- A class of internal bug surfaces that compile-time analysis
  CAN'T statically prove safe (e.g. parser-generated dispatch
  to runtime-discovered code paths).  Then the lib/server-style
  catch boundary may need to extend deeper.
- A real production loft program is run under a supervisor
  that genuinely benefits from in-process recovery (e.g. a
  long-running game server where restart cost is high), AND
  the library-level `serve(handler)` primitives can't cover
  the use case.  Then a narrow loft-side error-recovery
  primitive may be evaluated.
- User-code patterns emerge that NATURALLY want exception-style
  flow (e.g. deeply-nested validation chains where the typed-
  error mechanism becomes more boilerplate than the catch
  would).  Then the typed-error infrastructure may evolve;
  this decision specifically blocks try/catch for the
  internal-bug-recovery use case.

Pointer from the source: see PROBLEMS.md row P269 for the
specific incident this decision was crystallised in (server
process died on todo!() panic during the P268 fix work);
the compile-time check shipped 2026-05-13 in
`src/generation/mod.rs::output_function`.  Memory-system
mirror: `feedback_fail_at_startup_not_runtime.md`.

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
