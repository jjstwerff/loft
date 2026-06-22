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
  use 12B `Parts::DbRef` pointing at the live original (@P260 fix,
  `src/parser/vectors.rs::synthesize_closure_record`).  Mutations
  from either side are visible immediately.
- Captures of scalars whose bodies write to the capture are
  promoted to heap-owned cells via the phase-02d-iii.a type flip
  (`Type::Reference(__cell_<T>, vec![])` encoding).  The outer
  scope and the (single) mutating closure share the same cell —
  sharing the cell across SEVERAL closures is rejected at compile
  time since C74 (#314).
- Pure read-only scalar captures remain value-copy (Case A
  semantics — unchanged).

Case D ("aliased mutating") was decommissioned 2026-05-13: the
cell + auto-Reference machinery from phases 02-03 already gives
shared-state semantics, so no rejection was needed.  Design lives
in the closed plan README:
[plans/finished/22-mutable-closures/](plans/finished/22-mutable-closures)
— Case D's major finding sits in its phase file, and the
alternatives considered are in `DISCUSSION.md` alongside.

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

The loop-iteration aliasing bug (@P250 — `for { (q1, q2) =
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
known as of 2026-05-11.  @P250's fix lives in dep-tracking, not
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
All four are tracked under @PLN28 phase 4:

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
   `plans/28-error-messages/04-runtime-error-kinds.md
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
   compile-error per @P269.
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

Pointer from the source: see PROBLEMS.md row @P269 for the
specific incident this decision was crystallised in (server
process died on todo!() panic during the @P268 fix work);
the compile-time check shipped 2026-05-13 in
`src/generation/mod.rs::output_function`.  Memory-system
mirror: `feedback_fail_at_startup_not_runtime.md`.

---

## C68 — Keyed collections dedup on insert (`+=` AND `coll[key]=value`)

> **REVERSED & IMPLEMENTED (2026-05-21).**  The original decision below
> (close @P306 as a `+=`-append-vs-`[key]=`-upsert split) was reversed the
> same day on **new evidence**: the world-chunk index is `hash<Chunk[cx,cy,cz]>`
> and inserting a chunk where one already exists at a coord MUST replace, not
> stack a shadowed duplicate — so dedup-on-insert is a correctness requirement
> of the architecture, not a preference.  The risk concern was retired by
> doing it the safe way: hash/index dedup via `Stores::dedup_keyed`
> (find + free + unlink + reclaim, then add) in `insert_record`; sorted via an
> overwrite-on-`found` branch in `vector::sorted_finish`.  Full suite green
> (no consumer relied on duplicate-key append).  **Both `coll += [entry]` and
> `coll[key] = value` now dedup by key (latest insert wins).**  @P306 is
> closed by CODE, not by this decision.  The historical decision is kept below
> for the trade-off record.

### Question

For a keyed collection (`hash` / `sorted` / `index<T[K]>`), what
should `coll += [entry]` do when an entry with the same key already
exists?  Append a second record (current behaviour — `len` grows,
lookup returns the first), or replace (dedup)?  Filed as @P306 during
the plan-44 hash-semantics sweep.

### Evaluation

- A keyed collection implies key UNIQUENESS, so silently coexisting
  duplicate keys are a footgun: `coll[key]` returns the first, the
  duplicate is dead weight, and `len` over-counts.  This also wastes
  space — bad for the cache-locality goal that the chunk work is built
  around (a chunk's working set should fit L1/L2; bloat from duplicate
  keys pushes it out).
- BUT making `+=` dedup means modifying the hot insert path of THREE
  data structures (`hash::add`, `vector::sorted_finish`, `tree::add`)
  to find-and-replace on key collision — the @P295 minefield (the hash
  deep-copy/off-by-one bugs all lived here).  High regression risk for
  a change to a primitive every consumer uses.
- `coll[key] = value` (the @P305 `OpSetKeyed` upsert) ALREADY provides
  a correct, deduping, memory-bounded insert-or-replace for all keyed
  kinds, uniformly across local / field / `&`-param.  It is the
  compact, locality-friendly path the chunk work actually wants.

### Decision

**Closed (2026-05-21) as a two-operation split:**

- **`coll += [entry]`** is APPEND — the low-level primitive.  On a
  keyed collection a duplicate key is the caller's responsibility (it
  appends; no dedup).  Cheapest when keys are known-unique (the common
  bulk-build case — `build_index`, snapshot loads — never inserts a
  duplicate, so it pays nothing for a dedup check).
- **`coll[key] = value`** is UPSERT (insert-or-replace, deduped by the
  value's key) — the idiomatic map/set write.  Use it whenever keys may
  repeat or compactness matters.

This keeps the hot append path untouched (no @P295-class risk) while
giving a correct dedup path.  It mirrors the common library split
(`push`/`append` vs `insert`/`[]=`).  @P306 is closed by this decision,
not by a code change.

### Revisit when

- A consumer needs bulk dedup-append (many possibly-duplicate keys at
  once) and the per-element `coll[key] = value` loop is measurably too
  slow — then a dedup-aware bulk insert (or a `+=` dedup variant) can be
  evaluated, implemented in the per-kind add with the @P295 lessons in
  hand.
- Duplicate keys are shown to corrupt a downstream operation
  (iteration, deep-copy, removal) rather than merely waste space — then
  `+=` dedup becomes a correctness fix, not a preference.

Pointer from the source: PROBLEMS.md row @P306; spec + matrix in
[plans/finished/44-hash-semantics/](plans/finished/44-hash-semantics/README.md) (cases
C09/C10 document the append-no-dedup behaviour this decision blesses).

---

## C69 — `!x` on a non-boolean is a null test, not logical-not

### Question

`!x` where `x` is a non-boolean (integer, single, reference, …) reads as
*"is `x` null?"* rather than C-style logical-not.  Because the null sentinel
is **in-band** (`i64::MIN` for integer, byte `255` for boolean per @PLN17 / C73,
null `DbRef` for references), `!0` is `false` — `0` is a real value, not the
sentinel.  (This C69 decision is about *non-boolean* `!x`; boolean `!` is ordinary
coercing negation — both `null` and `false` give `!b == true`.)  A
crawler report (GitHub #253) hit this as a footgun: `f = 0; if !f { … }` never
runs, and a `gl_load_font` zero-handle check silently misses.  Should `!`
either coerce non-booleans C-style (`!0 ⇒ true`) or reject them as a
compile-time type error?

### Evaluation

Both proposed fixes break a load-bearing idiom:

- **C-style coercion (`!0 ⇒ true`)** would invert the meaning of every
  `!nullable` null test.  The stdlib `min` / `max` / `clamp` use `!both` to
  mean *"both is null"* (`default/01_code.loft`); coercion would silently
  change them to *"both is zero"*.
- **Compile error on `!<non-bool>`** rejects that same stdlib idiom, and any
  user code that writes `!handle` to mean "absent".

The asymmetry is **documented** (`LOFT.md` → *"`!value` asymmetry — read
carefully"*) and intrinsic to the in-band-sentinel memory model (the same
model that makes `C66`'s "return a sentinel, keep running" possible).  `!x`
as a null test is the *deliberate* shape: for a nullable `x` it is a real,
useful check; the surprise only arises when a reader imports C's
value-falsiness expectation.

What IS cleanly fixable is the **always-false** subset: `!x` on a statically
`not null` operand can never be the sentinel, so it is provably a no-op.  That
gets a compile-time **warning** (zero false positives — nullable operands and
boolean `!` stay quiet), mirroring the redundant-`??` warning.  Going further
(deprecating `!` on non-booleans, migrating the stdlib to `x == null`) was
weighed and declined: it removes a working idiom to chase a confusion that the
documentation + the always-false warning already address, and it churns the
stdlib's hottest helpers for cosmetic gain.

### Decision

**Closed — accepted design choice.**  Dated 2026-06-03.  `!x` on a
non-boolean stays a null test (sentinel-in-band).  Added safety net: a
`Level::Warning` at `!` of a statically `not null` non-boolean operand
("always false — `!x` tests whether x is null …"), landed in
`src/parser/vectors.rs::parse_single`.  Regression guards:
`gh253_bang_on_not_null_warns` (warns) and `gh253_bang_on_nullable_is_quiet`
(no false positive) in `tests/parse_errors.rs`.  GitHub #253 closed `by-design`
pointing here.

### Revisit when

A concrete loft program demonstrates that the null-test idiom causes a
recurring, hard-to-diagnose class of bugs that the always-false warning + the
documented asymmetry don't catch — AND a replacement (e.g. reserving `!` for
boolean with an explicit `x == null` null test) has been prototyped to show it
doesn't churn the stdlib's hot paths or surprise the in-band-sentinel model.
"I expected C semantics" is not sufficient evidence.

---

## C70 — No per-library IR snapshot / cache

**Question.** @PLN11 (Data-as-store) caches the compiler IR as mmap'able
store records.  Should a **library** be able to ship (or the toolchain
cache) *its own* IR snapshot independently, so a never-seen `use`
combination doesn't pay a full parse on first run?

**Evaluation.** Two independent reasons, both permanent:

- **A library cannot cleanly write its own IR.**  The IR is global-index:
  `def_nr` / `known_type` are absolute and parse-order-dependent (core and
  every `use`d lib append into one global `Data.definitions`).  A library
  snapshotted in isolation would have to be **relocated** by name into
  whatever prefix it lands in — the single brittlest mechanism in the whole
  caching design, and it optimizes only the least-common case (the first run
  of a brand-new lib-set).
- **The loft source is the better representation of a library's state.**  For
  distributing / versioning / inspecting a library, the `.loft` source — not a
  serialized IR image — is the right artifact.  And there is **no efficiency
  case** for a serialized per-library form: @PLN82 established by measurement
  that JSON (de)serialization is **not** faster than parsing natural loft
  source (both deserialize text into the same heap graph; ~15–24 ms load ≈
  ~11–23 ms parse).  A per-library IR cache would be a worse, harder-to-relocate
  stand-in for something the source already expresses well *and* parses just as
  fast.

The whole-bundle snapshot (core + a script's sorted lib-set, as one image with
internally-consistent indices) sidesteps relocation entirely and captures the
repeated-run win that actually matters for the dogfood consumers.

**Decision.** **Closed — declined.**  @PLN11 caches only **whole-prefix
bundles** (core stdlib; core + per-script lib-set); never independent
per-library IR.  Dated 2026-06-02 (first raised 2026-05-31).

**Revisit when.** A measured workload shows first-run parse of a never-seen
`use` combination is a real bottleneck (repeated runs already hit the bundle
cache), AND a relocation scheme is prototyped that doesn't reintroduce the
global-index brittleness — bring the parse-time profile and the relocation
design together, not separately.

Pointer from the source:
[plans/11-data-as-store/](plans/11-data-as-store/README.md)
§ What gets cached (per-library "dropped, not deferred").

---

## C71 — Native libraries compile, scripts interpret — the steady-state execution model

> *Build-plan seed:* the engine-host design exploration —
> [plans/18-engine-host/ENGINE_HOST.md](plans/18-engine-host/ENGINE_HOST.md) (tier model per
> LAVITION § Execution granularity, entry-gate probes, the main-loop IO contract,
> dogfood prior art from @PLN6 / @PLN51).

**Question.** As loft matures toward the lavition engine deployment model, what is
the correct execution model for the combination of stable libraries and the user's
own scripts?  Should everything compile to native artifacts, everything stay
interpreted, or should the two coexist?

**Evaluation.**

The split model — native for stable/published libraries, interpreted for the
user's active script — has a decisive set of advantages in loft's specific context:

- **Best of both worlds.** Native gives speed for heavy, stable code (libraries,
  engine); the interpreter gives fast iteration (no `rustc` per save) for the code
  under active edit.  This IS the lavition model: native engine + libraries,
  interpreted game scripts for rapid prototyping.
- **The shared-ABI advantage is unique to loft.**  The store / `DbRef` heap is
  already a shared ABI between the interpreter and native code — data crosses the
  boundary as `DbRef`s into the same `Stores`, with no marshalling or copying.
  This is the hard part in Python/JNI/FFI; loft gets it for free because
  both modes address the same `Stores` instance.
- **The mixed-mode dispatch primitive already exists.**  `OpStaticCall` →
  `library_names` → `extensions::wire_native_fns` → `try_dlsym` (and
  `native_packages` / `src/extensions.rs`) implements exactly this model today:
  interpreted bytecode calls compiled Rust via the same `#native` / `#rust`
  mechanism the stdlib uses.  This is not a new idea — it IS the current stdlib
  design, extended to user libraries.
- **The native backend's cross-mode byte-identical equivalence** (the @PLN11
  harness) guarantees a library behaves identically whether run interpreted
  (during development) or native (shipped).

**Performance implication — supersedes E2 / full zero-copy as the startup-perf
endgame (measured 2026-06-04).** The `bench_read_data_breakdown` profiling
(PERFORMANCE.md § Open work, E2 row) found warm-load cost is allocation-bound:
it is the materialisation of library bodies + variable tables into native
`String` / `Box<Type>`.  In the native-library model you NEVER materialise those
(libraries are native); you load only the small library interface (type schema +
function signatures + symbol map).  The allocation cost is AVOIDED, not
eliminated via a multi-week zero-copy rewrite.  E2 / full zero-copy therefore
drops to low priority / deferred-by-this-decision for perf purposes.  The store-IR
foundation and the `IrNode` handle still have architectural and self-hosting value;
the perf rationale for E2 is superseded.

**Cache design (local scope — 2026-06-04 decision).**

- **Purely local workspace for now** — no registry/per-target distribution, no
  WASM concerns.  First-use compile latency is accepted.
- Cache native artifacts **per library** (max reuse; native `init()`-sequencing
  composes independently-compiled libs at load — this is why per-library native
  artifacts work where per-library IR snapshots did NOT; see C70).
- Eviction = **idle-TTL with touch-on-use**: update an entry's last-use timestamp
  on every cache hit; a startup GC sweep deletes entries idle longer than
  `LOFT_CACHE_TTL_HOURS` (default 24 h).  The current
  `cache::prune_program_cache` (oldest-first size-cap) keeps its role as a
  runaway-backstop; idle-TTL is the primary policy.

**The build fingerprint — the correctness crux.**  A native artifact is generated
Rust that `extern`-links `libloft.rlib`, so it is valid only against the exact
loft build whose rlib it links.  The cache key MUST fold the **loft rlib CONTENT
hash** (already memoised once per process in `native_utils::native_cache_key` per
the BUILD2 design — `src/native_utils.rs`) + rustc version + target + feature-set.
It must NOT key on git-HEAD `BUILD_ID` (does not change on an uncommitted loft
rebuild) NOR on mtime (fragile / over-invalidates).

The rlib hash is the load-bearing term: the artifact links the rlib, not the
executable.  Treat them as one "loft build fingerprint"; the rlib hash is the
correctness guarantee.

Two enforcement points ("do both"):

1. **Nuke-on-recompile** — a startup self-check compares the current loft build
   fingerprint against a stored marker; on mismatch it clears the native artifact
   cache (fast cleanup of rebuild orphans; the recompile happens via external
   tools — cargo/make — so loft can only react at its next startup).
2. **Fingerprint folded into every per-artifact cache key** (the lazy backstop).

Together these make `make rebuild-native-cdylibs` obsolete.  Known offender to
migrate: the `@P341` native-PACKAGE rlib path still folds **mtime** (per
PERFORMANCE.md § BUILD2 notes) — that is the hole behind hitting the
"generated rust-code error" too often.

**Three-layer model.**

| Layer | Mechanism | Goal |
|---|---|---|
| rlib-hash in each artifact key | cache key invalidation | correctness — never link a stale artifact |
| nuke-on-recompile (startup marker check) | cache sweep | fast cleanup of rebuild orphans |
| idle-TTL GC (24 h, touch-on-use) | background eviction | space — genuinely-unused sets age out |

**Validation layer / developer-vs-customer framing.**  The build fingerprint is
eventually owned by a library validation layer: an artifact's validity =
content-hash · target · features · loft-build-fingerprint · (eventually)
signature.  The cache then becomes dumb storage + the idle-TTL janitor.  This
fingerprint serves different audiences on different timelines: loft DEVELOPERS
need it now (rlib changes constantly, often uncommitted → git-HEAD useless);
customers on RELEASES are covered today by `LOFT_VERSION`; customers on DAILY
BUILDS will need the fingerprint (same/rolling version but different codegen →
`LOFT_VERSION` fails).  Building it now for developers IS the
customer/daily-build mechanism.  Dovetails with reproducible builds (BUILD2
confirmed the rlib is byte-deterministic — `src/native_utils.rs`).

**Remaining risks and open points.**

- **Dispatch coverage** is the real engineering risk: simple `#native` functions
  work, but generics, closures crossing the boundary, and complex exported types
  are hard (see PACKAGES.md "What must be native" / the C-ABI boundary).  Needs a
  coverage pass.
- **Dev-interpret fallback**: a library under active edit should still interpret
  (no `rustc` per save); "always native" = once the library is stable/published.
- **WASM/browser is a different model** — no `rustc` at runtime, so `--html` stays
  whole-program AOT-to-WASM; native libs don't apply there.

**Sequencing.** Tactical now (developer-facing, local): per-library native artifact
cache with rlib-hash key + nuke trigger + idle-TTL; extract a single
`loft_build_fingerprint()` reusing BUILD2's memoised rlib hash
(`native_utils::native_cache_key`, `src/native_utils.rs`) so it has a clean seam
to move into the validation layer.  First concrete step: audit every
native-artifact cache key for mtime/git-HEAD usage (`@P341` is the known offender)
+ extract `loft_build_fingerprint()`.  Eventually (customer-facing): fold the
fingerprint into the library validation layer; becomes load-bearing when daily
builds ship.

**Goal alignment ([GOALS.md](GOALS.md)) — and the guardrails this decision carries.**

C71 is the **Purpose made concrete**: *do the hard plumbing so it's fun to pick
up* — libraries **done** (compiled, fast, stable) so the script is **written**
(interpreted, instant iteration, no `rustc` per edit).  The **shared store /
`DbRef` heap is the structural reason it can be goal-true**: one heap means one
memory model and zero-marshalling dispatch, which is what lets it satisfy E and F
*across* the boundary instead of bolting on a second runtime.

Per goal, in brief:

- **F (friction-free): aligned, with a hard constraint.**  The native-vs-interpret
  choice must be *automatic and invisible* (never a user annotation), and
  un-native-able code (generics / boundary-crossing closures) must **silently fall
  back to interpret, never error** — F's "absorb the cost, don't hand the user a
  form."  First-use compile latency is a fun-on-pickup cost (accepted), not an F
  violation (F is syntax/proof friction, not wait-time).
- **E (predictable memory): aligned *because of* the shared store.**  No separate
  native heap → a value's lifetime is its scope whether allocated by interpreted
  or native code (post-rc-removal, both free at scope end).  Constraint: the
  boundary must not grow an "except across a native call" rule — `LOFT_STORE_GUARD`
  must cover the mixed path.
- **A (soundness): both an instrument and a surface expansion.**  The build
  fingerprint is itself a Goal-A veil-lifter — it forbids the "stale artifact
  silently links after a build / rustc bump" failure A is *defined against*.  But
  C71 puts more, less-battle-tested native code across the interpret↔native
  boundary, so the sanitizer (Miri / ASan / `stack_align_guard`, esp. macOS-ARM
  alignment) must grow a leg for the *mixed* boundary — it **adds to the soundness
  floor, it does not clear it**.
- **D (parity): relies on it, adds a row.**  Built on the cross-mode byte-identical
  equivalence, but introduces a fourth combination — interp-script + native-lib
  must match *both* all-interp and all-native; the differential sweep must assert
  the mixed run agrees.  (WASM excluded — stays whole-program AOT, consistent with
  D treating backends as independent implementations of one semantics.)
- **C / B:** C (dogfood) strongly served — fast libs + fast script iteration is the
  dogfood ideal — but **gated on the dispatch-coverage gap** (the consumers use
  closures / generics).  B compatible and *enables* the daily-builds cadence (the
  fingerprint makes daily updates safe for customers).

**Guardrails (must hold, not later polish):**

1. **Sequencing vs the two floors.**  C71 is capability work, which GOALS.md gates
   on the soundness floor — and C71 *enlarges* that floor with the mixed boundary.
   So grow the A / D detectors to the mixed boundary **as part of** the work, not
   after; do not let C71 run ahead of the soundness floor it extends.
2. **F line:** the native/interpret decision stays automatic + silent-interpret
   fallback — zero user-facing surface, no "can't compile this native" error.
3. **E line:** the boundary stays memory-model-transparent; `LOFT_STORE_GUARD` is
   extended to the mixed path and the exceptionless rule holds.

**Decision.** **Native libraries compile once (cached per rlib-hash fingerprint),
user scripts interpret.** This is the steady-state execution model loft optimises
toward.  Dated 2026-06-04.

Full narrative, risks, and sequencing: [BROADENING.md § Native-library execution
model](BROADENING.md#native-library-execution-model--the-steady-state-design).

**Status (2026-06-04).**  The dispatch **mechanism is proven end-to-end** — an
interpreted script calling an auto-generated, auto-compiled cdylib over the shared
`*mut Stores` (zero-marshalling), across **all common types both directions**
(scalars, vectors, structs, text, plain+data enums, keyed `sorted`), plus the
source-form lean interface and the `use`-shaped core (`mark_native_exports` →
`build_shared_cdylib` → `wire_shared_native_fns`, a normal library fn dispatched
with no `#native`).  See [@PLN11 Arc N](plans/11-data-as-store/README.md#arc-n--native-library-execution-model-c71-build-out)
and its **§ Landing sequence** for the ordered, landable path from here to the
steady state (A: wire `use`; B: make it invisible; C: soundness sweep; D: polish).

**Revisit when.** The dispatch-coverage audit reveals that the C-ABI boundary
cannot express a critical class of library API without marshalling cost that
erases the native-speed advantage — and a concrete measurement shows this matters
for a real consumer.  "WASM needs native libs" is not a trigger (WASM is a
separate model, always AOT).

---

## C72 — REPL session resume does not persist RNG generator state

**Question.** Should REPL auto-resume snapshot and restore the random
generator's internal state, so the random stream continues identically across a
stop/resume (exact-deterministic resume)?

**Evaluation.** Restoring saved RNG state makes the stream reproducible from the
saved image: anyone who reads the session file could predict or replay future
`random()` outputs, and a restored state re-issues values that callers assumed
were fresh.  That is a predictability / security hazard for any use of
randomness (tokens, nonces, shuffles), paid for a feature already covered
another way — users who want a reproducible stream set an explicit seed
(`random_seed`).  Drawn random *values* already restore exactly via the store
image (they are ordinary stored values); only the generator's forward position
is at issue.  The PCG state also lives in the `random` cdylib, not a store
(src/ops.rs), so persisting it would be added surface, not a free side effect.

**Decision.** **Closed — declined (2026-06-08).**  Resume restores stored values
verbatim but does NOT persist or restore RNG generator state; on resume the
generator continues fresh (re-seeded from entropy, as on any launch).
Deterministic streams stay an explicit-seed opt-in.  This also keeps the session
image free of generator state, narrowing what a saved image exposes.

**Revisit when.** A concrete, non-security use case needs byte-identical RNG
continuation across resume that an explicit seed cannot satisfy.

---

## C73 — `boolean` is three-state (false / true / null); `==` is raw, truthiness coerces

**Question.** `boolean` was the only common-value scalar whose zero-value collided with
its null sentinel (null *was* `false`), so a nullable boolean couldn't distinguish
"absent" from "false" — unlike `integer` (0 ≠ `i64::MIN`), `float`, `text`, plain `enum`.
A `hash → boolean` map couldn't express absent / false / true.  Should boolean become
three-state, and if so, what are the comparison and coalescing semantics?

**Evaluation.** A boolean is stored in one byte — the same storage class as a 2-variant
plain enum, and plain enums already reserve byte `255` for null.  So the third state has
room for free; boolean was the lone byte-scalar flattening to 2-state.  Two semantic
candidates for `==`:

- **A — raw compare** (chosen): `==`/`!=` compare the raw byte, so `null == false` is
  `false` and `b == null` is the dedicated null test.  Truthiness contexts
  (`if`/`while`/`!`/`&&`/`||`) coerce `null → false`.  This is **exactly what `integer`
  already does** (`0 == null` is `false`; `n == null` is the null test; `if n` treats
  null as falsy) — so it makes boolean *consistent* with every other type.
- **B — coerce in `==`** (rejected): `null == false` would be `true`, forcing `== null`
  to be a special raw test bolted onto a coercing `==`.  That introduces a *new*
  inconsistency (boolean `==` coerces, integer `==` doesn't) — the opposite of the goal.
  Evidence settled it: `0 == null` is `false` for integer, so A is the consistent choice.

**Decision.** **Three-state boolean, design A.**  Dated 2026-06-10 (@PLN17).
- Representation: `false`=0, `true`=1, `null`=255 (byte), held/distinguished everywhere a
  boolean lives — locals, params, returns, tuples, struct fields, vector/keyed elements.
  A `boolean not null` is 2-state.
- `==`/`!=` are **raw** (distinguish null); `b == null` is the null test; truthiness and
  `&&`/`||`/`!` **coerce** `null → false`; `??` / `?? return` work (null-check is `== 255`,
  not truthiness — so `false ?? x` keeps `false`).
- Native mirrors the interpreter via the `u8`(storage)/`bool`(expression) two-form split
  (like `text`'s String/Str); heap storage mirrors plain-enum byte storage.
- **Supersedes the #256 guard cluster** (which *rejected* `null`/`??`/`== null` on
  boolean because false was indistinguishable from null — no longer true).
- Cross-mode byte-identical on both backends; regression: `tests/scripts/292-pln17-three-state-boolean.loft`.
  Full design + decision history: [`plans/17-three-state-boolean/`](plans/17-three-state-boolean/README.md).

**Revisit when.** Never, barring a fundamental change to the in-band-sentinel memory
model.  (Open, non-boolean-specific tail: the construction-vs-parse default for an
*omitted* field — `S{}` gives the zero value, `parse` gives null — affects integer too;
tracked separately, not part of this decision.)

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

## C74 — A mutated scalar may be captured by only ONE closure

**Question.** Should a bare scalar local that one closure mutates be
sharable with other closures (`run2(fn() { print(t) },
fn() { t = t + 1 })`) — JS/Python-style shared upvalues?

**Evaluation.** The shape only works through shared heap cells
(`__cell_<T>`) referenced by several closure records at once, and the
store model gives that sharing no defined owner: plan-57 removed the
ref-count, so `free_named` frees the cell at the FIRST record's death
and silently no-ops for the rest ("first death wins") — a latent
use-after-free class whenever one sharer dies early.  The parse-side
is equally fragile: closure-record attribute types freeze at each
lambda's pass-1 epilogue while the boxing decision (`scalars_to_box`)
accumulates until the parent's body end, so a reader lambda parsed
before the writer baked in the unboxed layout and crashed at runtime
(#314, interp CONST_STORE panic / native codegen failure).  No
consumer needs the shape: the @PLN18 kernel that surfaced it
immediately preferred a struct (`w.n`), which also makes the sharing
visible at every use site.  First worked example of
[GOALS.md § "Stability trumps features"](GOALS.md#stability-trumps-features).

**Decision.** **Closed — rejected at compile time.**  Dated
2026-06-10.  When more than one closure captures a scalar that any
closure mutates, the parser reports *"sharing a mutable variable
between closures is not supported — hold the shared state in a struct
field instead"* (`Parser::reject_shared_mutable_scalar_captures`,
`src/parser/vectors.rs`; chokepoint: the parent's pass-1 body end,
where `scalars_to_box` is final).  Kept as supported: the
single-closure accumulator (`fn() { sum = sum + x }` — one record,
one owner, sound), read-only sharing of a scalar across any number of
closures, and struct-field state captured by any number of closures.
Supersedes the C38 plan-22 addendum sentence "the outer scope and all
closures share the same cell" for the N≥2 case.  Regression guards:
`tests/issues.rs::issue_314_scalar_shared_by_two_closures_rejected`
and `::issue_314_single_closure_accumulator_still_works`.

**Revisit when.** A real consumer presents a shape that is materially
clumsier as a struct AND the cell gets a single defined owner first
(e.g. the parent frame owns the cell and records never cascade-free
it).  This is a default to keep in mind, not a hard line — reevaluate
on evidence.

## C75 — Closure-carrying struct values are frame-bound

**Question.** May a struct holding a capturing closure leave the function
whose frame owns the captures — be returned, written into an argument's
field, or stored in a collection?

**Evaluation.** The closure record holds raw 12-byte DbRefs into the
constructing frame's stores (Reference captures and `__cell` scalar boxes
alike).  Every escape route copies the record's bytes — `OpClaimChildRec`
clones the DbRefs but nothing transfers the stores they name — so the frame
frees them at return and the free-bitmap hands the slots to the next
allocation: the escaped closure then silently reads and writes an unrelated
live object (#318; probed matrix in the issue, probes in
`/tmp/p_followups/e*.loft`).  Within-frame use — locals, downward argument
passing, #313's whole matrix — is sound.  A real ownership transfer through
the deep copy is a substrate design (cross-store fix-ups, native mirroring);
no consumer needs the escape today.

**Decision.** **Closed — rejected at compile time.**  Dated 2026-06-10.
Three sinks reject on the transitive `Parser::type_carries_closure`
predicate (derived from the registered DB layout, order-stable): returning a
closure-carrying struct type (`definitions.rs`, the pass-2 body hook),
writing a capturing closure into a struct rooted at an argument
(`set_field_check`'s fn arm), and collections of closure-carrying structs
(`sub_type` — extends the plan-15 CLOSED `vector<capturing fn>` cell, which
a struct wrapper had silently bypassed).  Struct assignment is copy-at-value
(C38), so a local alias cannot smuggle a write past the argument check
(probe e10).  Returning a BARE capturing closure stays supported on interp
(case-C factory transfer); its native divergence is #323.  Regression
guards: `tests/issues.rs::issue_318_*`.

**Revisit when.** A consumer needs factory-built closure-holding structs AND
the deep-copy path gets a designed ownership transfer (claim the captured
stores into the host, or re-point the record at host-owned copies) —
verified on both backends.

## C76 — Selective imports group with `()`, not Rust-style `{}`; flat comma list dropped

**Question.** How does a `use` import multiple names from one library — a flat
top-level comma list (`use lib::a, b, c;`), Rust-style braces
(`use lib::{a, b, c};`), or parentheses (`use lib::(a, b, c);`)?

**Evaluation.** The flat list reads poorly — `b`/`c` don't visually bind to
`lib::`, and at a glance look like separate statements.  `{}` is loft's
block/struct-literal delimiter; reusing it for imports invites confusion with
struct construction and a future `use lib::{ … }` block.  `()` is loft's existing
arg-list/grouping delimiter, reads as "these names belong to `lib::`", and is
lighter than braces.  Per-name `as` aliasing (@PLN22 Phase 3) composes inside any
of the three.

**Decision.** **Grouped `use lib::(a [as x], b, c);` (@PLN22 Phase 4, 2026-06-14).**
A single `use lib::name [as bind];` is unchanged; multiple names MUST be
parenthesised; the flat `use lib::a, b;` list is a hard error ("import multiple
names with parentheses") that still binds the names (recovery).  Rust-style `{}`
braces are NOT adopted — `{}` stays reserved for blocks/structs.  Sole flat-list
site migrated; tests `imports::pln22_phase4_grouped_import` /
`pln22_phase4_flat_list_rejected`.

**Revisit when.** A concrete need arises for nested/path grouping that `()` can't
express (e.g. `use a::(b::c, d)`), with a parse that doesn't collide with the
struct-literal or call grammar.

## C77 — Binding ownership: value-semantics by default, copy/share/move chosen by analysis

**Question.** When `a = x` / `a = x.f` / `a = x.v[i]` binds from a value backed by
another store, is the binding a COPY (independent value), a VIEW (alias), or chosen
per binding *form*? loft today does all three by form — whole-value eager copy, the
#415 struct-field copy-on-bind, the `a = x.v[i]` element view — the copy-vs-view
inconsistency #426 surfaced.

**Evaluation.** Three candidate invariants:
- *View-by-default* (status quo): element/field reads alias, whole-value copies. The
  `=` ambiguity is permanent and non-local — "is this a copy?" can't be read off the
  line — and the split manufactures the store-lifetime bug class (Cluster A, #415, #426).
- *Copy-always*: uniform but pessimal (eager copies everywhere) and still cannot
  express write-through.
- **Value-semantics by default, copy/share/move chosen by a path-sensitive
  liveness+mutation analysis** ([OWNERSHIP_MODEL.md § The law](OWNERSHIP_MODEL.md)):
  observably every binding is an independent value; the compiler *shares* while no
  aliasing write is possible and *moves* when the source is dead. Uniform across all
  forms — the source is just a path expression.

**Decision — ACCEPTED as the direction (2026-06-22).** Value-semantics by default;
copy-vs-view is not a language distinction but an implementation choice the borrow
analysis makes per binding. Genuine write-through uses the **`&` binding**
(`cells = &chunk.ck_cells`) — the *same* `&` notation loft already uses for
`&vector<T>` parameters (writes propagate to the source), now allowed at a local
binding; **no lifetime annotations** (the borrow checker infers source-outlives-binding
from scope, exactly as it already does for `&` params — C38's objection was to
reference *types*, not this binding *notation*). `=` and `&` read ONE analysis and make
the same "can I share?" decision, differing only in the observable contract: `=`
**shares as an efficiency pass** (copy-on-write — sharing is the fast default, the copy
materialises only when a write diverges), `&` makes the link the contract. The
implicit-write-through consumers (hex_world `set_cell` / p379) migrate mechanically by
adding `&`. This is the concrete content of the OWNERSHIP_MODEL beacon and the stated
target for the borrow-checker work — it collapses the per-form special cases and closes
the store-lifetime bug class by construction. Staged: **share-or-copy first**
(correctness core, dissolves #426); **move-when-dead later** (move on a field/element
is a *partial* move); the **`&`-binding** is loft's existing param mechanism extended
to local bindings.
Consistent with C64 (tuple struct-ref elements already use MOVE, not copy).

**Revisit when.** The falsification probe fails — rewriting a real write-through
consumer (`set_cell` / p379) with `&` proves *not* expressible or badly unergonomic
(implicit write-through through deep nesting turns out to be a needed idiom `&` can't
spell cleanly). That would mean value-semantics-by-default is the wrong default and the
view model should stay, documented in INCONSISTENCIES.md.
