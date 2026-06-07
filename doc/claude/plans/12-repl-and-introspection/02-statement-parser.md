<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 02 — Statement-level parser entry

**Status: in progress.**
- **Increment 1** (2026-06-07): `Parser::statement_incomplete` — the "read more
  lines?" detector.
- **Increment 2** (2026-06-07): `parse_statement` + `ParseResult` for top-level
  **definitions** (`struct`/`enum`/`fn`/`type`), via incremental append against
  the live session, with transactional rollback (`Data::rollback_to`) on error.
  `NeedMore` for incomplete input.  Cross-statement references work (a later
  statement sees an earlier def).  Tests in `tests/parser_statement.rs`.
- **Increment 3** (2026-06-07): bare expressions / calls / statements wrap into a
  synthetic runnable `fn repl_<n>()` (`Parser::starts_top_level_def` routes a
  definition vs the wrapper), so they parse and `Ready.entry_def_nr` points at a
  runnable fn.  Tests in `tests/parser_statement.rs`.
- **Remaining**: cross-input **local persistence** — the `__repl_session` struct
  below.  A local declared in a wrapped statement does not yet survive to the
  next input.  This interlocks with phase 03's runtime (the session instance
  must persist across `reset_for_repl`) + needs new-local type inference, so it
  is best built together with phase 03 rather than as a parse-only step.

## Validated constraints (2026-06-07)

Three load-bearing claims in the original design were checked against the code
and corrected — build on these, not the first-draft assumptions:

1. **`parse_str` discards the stdlib.**  It calls `data.reset()` and parses
   *only* the given string, so it cannot be used naively for an incremental
   REPL parse — every prior stdlib/user definition would vanish.
2. **The two-pass model resets `data` before *both* passes** (`Parser::parse`).
   Pass 2 needs every definition present to resolve references, so "append one
   def to the live `data`" fights the design.  The realistic base strategy is:
   **reset → reload the stdlib (cached via the D2b stdlib cache) → re-parse the
   accumulated REPL session source.**  The `__repl_session` struct below is the
   local-persistence layer *on top of* that re-parse — the same picture, just
   spelled out against how the parser actually works.
3. **The lexer has no persistent EOF-bracket-depth API** (the original step 5
   assumed one existed).  Increment 1 adds `Parser::statement_incomplete`, a
   pure scanner over the input string instead.

## Goal

A `Parser::parse_statement(input: &str) -> ParseResult` API that:

- Accepts a single loft statement (struct/enum/fn definition,
  `let`-style binding, expression, assignment, `for`/`if`/`while`
  block).
- Reuses an existing `Parser`'s `data` + `database` so previously-
  defined names are visible.
- Recovers from incomplete input (parser indicates "need more
  input") so a multi-line block can be assembled across stdin
  reads.
- Returns either a fully-parsed IR ready for codegen + execute, or
  a structured error.

This is a hard prerequisite for the REPL.  No external behaviour
change for file-level loft programs.

## Design

### API shape

```rust
pub enum ParseResult {
    /// Fully parsed; the parser registered any new top-level def
    /// in `data` and built the body IR ready for codegen.
    Ready { entry_def_nr: u32 },
    /// Input ends mid-construct (open `{`, open `(`, multi-line
    /// string).  REPL prompts for more input and re-calls with the
    /// concatenated string.
    NeedMore,
    /// Parse error; `data` and `database` are restored to the
    /// pre-call state (transactional rollback).  REPL prints the
    /// error and re-prompts.
    Error(Vec<Diagnostic>),
}

impl Parser {
    /// Parse a single REPL input.  Persists `data` + `database`
    /// across calls; resets `lexer` and per-statement `vars`.
    pub fn parse_statement(&mut self, input: &str) -> ParseResult { … }
}
```

### Two-pass adaptation

File-level parsing today (`Parser::parse`):
1. Pass 1: define every top-level construct (struct/enum/fn etc.).
2. Pass 2: parse each fn body + codegen.

For a single statement, three cases:

| Statement shape | Passes needed |
|-----------------|---------------|
| Top-level def (`struct Foo { … }`, `fn bar() { … }`, `enum E { … }`, `type T = …`) | Both — register the def AND parse its body. |
| Expression / assignment (`x = 1 + 2`, `print(x)`) | Pass 2 only — wraps in a synthetic `__repl_N` fn for codegen. |
| Mix (`let f = fn(x) -> x + 1; f(3)`) | Both passes via the synthetic fn wrapper. |

The parser dispatches by peeking the first lex token:
- `struct` / `enum` / `fn` / `type` / `pub` → top-level def path
  (existing `parse_struct` / `parse_enum` / `parse_function` /
  `parse_typedef` re-used).
- Anything else → wrap-in-`__repl_N`-fn path.

### Synthetic `__repl_N` wrapping

Each non-def statement gets compiled as the body of a synthesised
fn `n___repl_N` (where N is a monotonically incrementing
counter on the `Parser`).  The wrapper fn's body is the
statement; its locals carry over via a side channel (see below).

After codegen, the REPL invokes `n___repl_N` and discards the fn
def from the registry (or keeps it for debugging — TBD per phase
04 design).

### Local-variable persistence

REPL inputs:
```
loft> x = 1
loft> y = x + 2
loft> print(y)
```

Each line creates `n___repl_1` / `n___repl_2` / `n___repl_3`
synthetic fns.  But `x` defined in input 1 must be visible to
input 2.

**Implementation choice**: maintain a top-level `__repl_session`
struct.  Each input that creates a new local appends a field to
`__repl_session` and rewrites the assignment to write into that
field.  Subsequent inputs read from the same struct.

This sidesteps the "scope-extends-across-stack-frames" problem
(every input gets its own stack frame; the persistent state lives
in the database, not on stack).

```loft
// Input 1: x = 1
struct __repl_session { x: integer }
fn n___repl_1(s: __repl_session) {
    s.x = 1
}

// Input 2: y = x + 2
struct __repl_session { x: integer, y: integer }   // appended
fn n___repl_2(s: __repl_session) {
    s.y = s.x + 2
}

// Input 3: print(y)
fn n___repl_3(s: __repl_session) {
    print(s.y)
}
```

The session struct evolves across inputs.  Stores hold one
instance of `__repl_session`; each REPL call passes its DbRef.

**Trade-off**: appending fields to an existing struct may relayout
older fields.  Two options:
- **(a)** Re-build the struct each time and re-copy old field
  values → expensive but simple.
- **(b)** Reserve generous slot space for each field at first
  declaration → fragile if user defines many vars.

Phase 02 picks (a) for simplicity; phase 03 may revisit.

### Incomplete-input detection

`Lexer` tracks bracket nesting.  `parse_statement` returns
`NeedMore` when:
- Lex cursor is past EOF AND bracket-depth > 0.
- Last token is a binary operator (e.g., `1 +` then EOF).
- Inside a multi-line string literal.

The REPL accumulates the previous input + new line on `NeedMore`
and re-calls.  Recovery: a Ctrl-C / blank line resets the
accumulator.

### Transactional rollback on error

Parse errors must NOT corrupt `data` or `database`.  On `Error`,
the parser undoes:
- New definitions added to `data.definitions` (truncate to
  pre-call length).
- New entries in `data.def_names` (delete keys with values >=
  pre-call definition count).
- New entries in `database.types` and `database.names`.
- New attributes added to existing definitions.

Implementation: snapshot the lengths at `parse_statement` start;
truncate on error.  Costs are O(N) in items added by the
statement, which is small.

## Implementation outline

| Step | Files | Effort |
|------|-------|--------|
| 1. `ParseResult` enum + `parse_statement` skeleton — **DONE** | `src/parser/mod.rs` | XS |
| 2. Top-level def dispatch (struct/enum/fn/type) — **DONE** via incremental `parse_str` append (no dispatch needed; `parse_str` already routes each def shape), wrapped in transactional rollback | `src/parser/mod.rs` | S |
| 3. Synthetic `fn repl_<n>()` wrapper for non-def statements — **DONE** | `src/parser/mod.rs` | S |
| 4. `__repl_session` struct evolution (append-field-on-new-local) — **remaining**, paired with phase 03 (runtime persistence + new-local type inference) | `src/parser/mod.rs`, `src/parser/objects.rs` | M |
| 5. Incomplete-input detection — **DONE** (`Parser::statement_incomplete`, a pure string scanner; the lexer had no reusable bracket-depth API) | `src/parser/mod.rs`, `tests/parser_statement.rs` | XS |
| 6. Transactional rollback — **DONE** (`Data::rollback_to`: truncate `definitions` to the pre-call count + `rebuild_indices`) | `src/parser/mod.rs`, `src/data.rs` | S |
| 7. Tests — unit tests for each statement shape (def, expr, assign, multi-line) | `tests/parser_statement.rs` (new) | S |

## Tests

### Unit tests for each statement shape

```rust
#[test]
fn parse_statement_top_level_struct() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    let r = p.parse_statement("struct Pair { a: integer, b: integer }");
    assert!(matches!(r, ParseResult::Ready { .. }));
    assert!(p.data.def_nr("Pair") != u32::MAX);
}

#[test]
fn parse_statement_local_binding_persists() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse_statement("x = 42");
    let r = p.parse_statement("y = x + 1");
    assert!(matches!(r, ParseResult::Ready { .. }));
    // Verify __repl_session has both x and y fields.
}

#[test]
fn parse_statement_incomplete_returns_need_more() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    let r = p.parse_statement("fn dbl(x: integer) -> integer {");
    assert!(matches!(r, ParseResult::NeedMore));
}

#[test]
fn parse_statement_error_does_not_pollute_state() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    let pre_count = p.data.definitions();
    p.parse_statement("struct Foo { ??? }");   // syntax error
    assert_eq!(p.data.definitions(), pre_count, "rollback failed");
}
```

### Integration test: simulated session

A test that drives a multi-input session via repeated
`parse_statement` calls and verifies the final state matches the
equivalent file-mode parse.

## Acceptance criteria

1. Single-statement parsing works for every top-level def shape
   (struct, enum, fn, type, constant) and for expressions /
   assignments inside the implicit `__repl_session`.
2. Incomplete input returns `NeedMore` cleanly; concatenating the
   continuation parses successfully.
3. Parse errors leave `data` + `database` byte-identical to the
   pre-call state.
4. File-mode parsing (`Parser::parse(file, default)`) is
   byte-equivalent to running every line of that file through
   `parse_statement` in sequence (modulo `__repl_session` wrapping
   for non-def lines).
5. Full `cargo test --release` suite green; no existing test
   touches `parse_statement` so the change is purely additive.

## Effort

**M (~1–2 days).**  The transactional rollback and `__repl_session`
struct evolution are the meaty pieces.  Step 4 (struct evolution)
may need a sub-design once concrete: appending fields to an
existing struct can break offsets for any existing user-data.
The REPL's `__repl_session` is freshly created each time, so the
issue is contained.

## Risk

- **Struct relayout cost** if users define many locals.  Phase 02
  picks the rebuild-on-each-new-local approach; phase 03 may
  swap for an arena-style "reserve N slots" model if benchmarks
  show overhead.
- **Two-pass interaction** with `__repl_session` evolution.  Each
  call to `parse_statement` runs both passes if a new top-level
  def appears; this means re-parsing every previously-defined
  function body each time.  Mitigation: cache per-fn IR after
  first compile; skip re-codegen if no def the fn references
  changed.  Defer optimisation until profiling shows it matters.

## Out of scope

- **Type inference for `let` without annotation** beyond what loft
  already supports.  This phase doesn't change type-inference
  rules.
- **Module-level `use` imports** in REPL inputs.  Deferred — the
  REPL starts with the `default/` stdlib only.

## See also

- [00-baseline.md](00-baseline.md) — file-level parser API survey.
- [03-state-reset-and-append.md](03-state-reset-and-append.md) —
  next phase, wires this parser entry to the runtime.
- `src/parser/mod.rs::parse` — the file-level entry this phase
  decomposes.
