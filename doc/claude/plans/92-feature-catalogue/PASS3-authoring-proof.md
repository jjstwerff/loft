<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN92 Pass 3 — authoring proof (go/no-go evidence)

**Purpose.** @PLN92's whole justification is gated on one question ([README §
Success criterion](README.md#success-criterion--gated-on-doc-improvement)): does
authoring self-contained per-feature docs produce *significantly better* language
documentation? This is the cheap falsification test — three features authored in full
(`## What it is` / `## How it aids you` / `## Example`), examples run cross-backend, put
beside today's LOFT.md.

**Method.** Authored **@F17** (named args), **@F29** (match), **@F22** (closures). Each
example was run on **both** backends (`--interpret` + `--native`) and produced identical
output — so they qualify as the tested examples the format requires.

## Verdict

**Significantly better for the *terse* half of the catalogue; a modest reformatting for
the already-detailed half.** The uniform wins are real and apply to every feature:

- an explicit **value** statement — LOFT.md usually says *what*, rarely *why you'd want it*;
- a **complete, tested** example — today's are often un-runnable fragments;
- consistent structure across all features; single source (docs + `tests/docs` generate from the issue).

But the format does **not** make docs *deeper* — depth still comes from writing. For
already-rich sections (match, closures) the gain is polish + a value line + a tested
example; to match their depth the authored issue must be as long (self-contained allows
it — that's *more writing, not less content*). So: **uniform, incremental improvement,
biggest where docs are thinnest.**

Roughly half the catalogue is terse like @F17 — whose current LOFT.md example has **no
function body and prints nothing** — so the aggregate lift is real, but it is an
**authoring grind proportional to 56 features**, not a free structural upgrade.

**Go/no-go:** worth it *iff* the goal is consistency + explicit value + tested examples +
single-source across all 56. If deeper docs were expected from the *format alone*, it
won't deliver that — depth is writing, feature by feature.

## Per-feature comparison to today's LOFT.md

| Feature | LOFT.md today | Authored delta |
|---|---|---|
| **@F17** named args | 3 sentences + a fragment (`connect` has **no body**, prints nothing) | **large** — value statement + complete runnable+verified example + self-contained |
| **@F29** match | already detailed (enum destructure, guards, exhaustiveness nuance) | **modest** — value statement + one tested example + uniform shape; *less* comprehensive unless lengthened to match |
| **@F22** closures | already detailed (per-type capture rules, cross-scope, limitations) | **modest** — same; the win is polish + a tested example |

## The three authored drafts

### @F17 — Named arguments + default parameter values

**## What it is** — Any parameter can be passed by name (`name: value`) instead of by
position, and any parameter may declare a default (`= expr`) used when the argument is
omitted. Positionals come first; after the first named argument every argument must be
named; any skipped parameter must have a default.

**## How it aids you** — Removes two daily frictions: memorising argument order, and
passing a long tail of arguments just to reach the one you care about. Call sites
document themselves (`connect(host: "db", port: 5432)`), and adding a new optional
parameter with a default never breaks existing calls.

**## Example** *(identical on `--interpret` and `--native`)*
```loft
fn connect(host: text, port: integer = 8080, tls: boolean = true) -> text {
    "{host}:{port} tls={tls}"
}
fn main() {
    println(connect("example.com"));              // example.com:8080 tls=true
    println(connect("example.com", tls: false));  // example.com:8080 tls=false
    println(connect(host: "db", port: 5432));     // db:5432 tls=true
}
```

### @F29 — Pattern matching (match)

**## What it is** — `match` chooses one arm by testing a value against patterns and
evaluates to that arm's expression. Patterns cover enum variants (with field
destructuring), scalar literals / ranges / `null`, or-patterns (`a | b`), and a `_`
wildcard; any arm may add an `if` guard. The compiler requires exhaustiveness — every
case covered, or a `_`.

**## How it aids you** — Replaces long `if/else if` ladders and manual variant checks
with one exhaustive, readable dispatch — and exhaustiveness means adding a new enum
variant turns every incomplete `match` into a *compile error*, not a silent
fall-through. As an expression it also assigns its result directly instead of mutating a
variable across branches.

**## Example** *(both backends)*
```loft
fn grade(score: integer) -> text {
    match score {
        null     => "absent",   // null is a first-class pattern
        90..=100 => "A",        // inclusive range
        80..90   => "B",        // exclusive range
        _        => "other"     // required wildcard
    }
}
fn main() {
    println(grade(95));  // A
    println(grade(85));  // B
    println(grade(50));  // other
}
```

### @F22 — Closures & lambdas

**## What it is** — A lambda (`fn(x: T) -> R { … }`) is an anonymous function value you
can store, pass, and return. When its body references a variable from the enclosing
scope it becomes a closure: the referenced values are **copied into the closure at the
point it is defined** (value semantics, like Rust `move`). A function may return a
closure; the captured values travel with it.

**## How it aids you** — Build behaviour on the fly — callbacks for `map`/`filter`,
small strategies, functions that manufacture other functions (`make_adder(5)`) — without
a named function for each. Copy-at-definition means the closure's behaviour is fixed when
created: later reassignment of a captured variable can't change it out from under you,
removing a class of "why did this callback change?" bugs.

**## Example** *(both backends)*
```loft
fn make_adder(n: integer) -> fn(integer) -> integer {
    fn(x: integer) -> integer { n + x }   // captures n by value
}
fn main() {
    add5 = make_adder(5);
    println("add5(10) = {add5(10)}");      // add5(10) = 15
    greeting = "Hello";
    greet = fn(name: text) -> text { "{greeting}, {name}" };
    greeting = "Bye";                      // does NOT affect the closure
    println(greet("world"));               // Hello, world  (captured at definition)
}
```

## If the answer is GO

These three drafts become the first authored issues (their examples extracted to
`tests/docs/@F17.loft` etc. by the strand-3 automation). Then work down the catalogue,
prioritising the **terse** sections where the lift is largest.

## See also

- [README.md § Success criterion](README.md#success-criterion--gated-on-doc-improvement) — the ROI gate this tests.
- [README.md § Backfill checklist](README.md) — Pass 3 is the authoring pass.
- `@PLN92` — the tracker issue.
