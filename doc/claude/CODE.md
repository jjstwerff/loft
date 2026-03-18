# Code Rules

Rules for all Rust and loft code in this project.

---

## Contents
- [Naming](#naming)
- [Functions](#functions)
- [Doc Comments](#doc-comments)
- [Test Suite (`tests/docs/`, `tests/scripts/`)](#test-suite-testsdocs)
- [Clippy and Formatting](#clippy-and-formatting)
- [Null Sentinels](#null-sentinels)

---

## Naming

- Names of functions, variables, arguments, and fields must be self-documenting — short but unambiguous.
- User functions in loft are stored with an `n_` prefix: `data.def_nr("n_foo")`, not `data.def_nr("foo")`.
- Native stdlib functions follow the scheme `n_<func>` (global) or `t_<LEN><Type>_<method>` (method, LEN = type name length). Example: `t_4text_trim` for `text.trim()`.
- Operators use `OpCamelCase` in loft source → bare `snake_case` in Rust (`fill.rs`), without any prefix. Exception: `OpReturn` → `op_return`, because `return` is a Rust keyword.

## Functions

- One algorithm per function. Extract helpers to avoid duplication.
- Group fields that always travel together into a struct.
- No functions longer than ~50 lines; split if the cognitive complexity warning fires.

## Doc Comments

- Describe *why* to call the function (preconditions, trade-offs, when to use), not *what* it does.
- Inline comments only where the algorithm is non-obvious. Avoid restating what the code says.

## Test Suite (`tests/docs/`, `tests/scripts/`)

- Each `.loft` file is a living language example as well as a test.
- Every section should have a `// comment` explaining what it exercises and why.
- Tests use `assert(condition, "message")` — the message is the failure label.
- `@NAME: title` and `@TITLE: description` headers are required for documentation generation.

## Clippy and Formatting

- No clippy warnings. The crate root sets `#![warn(clippy::pedantic)]`.
- `cognitive_complexity` (from `clippy::nursery`, not included in `pedantic`) is used selectively; suppress it per-function with `#[allow(clippy::cognitive_complexity)]` only for functions that are structurally complex by necessity (e.g. per-opcode match arms).
- Use `#[allow(clippy::...)]` only when the linter false-positives; always include a comment explaining why.
- Code is formatted with `rustfmt`. No manual formatting overrides.

## Null Sentinels

- Integer null: `i32::MIN`. Long null: `i64::MIN`. Float null: `f64::NAN`. Reference null: `store_nr == 0 && rec == 0`.
- All arithmetic operations must propagate null (if either operand is null, result is null).
- Never use `0` as a sentinel for integers or references in new code.

---

## See also
- [TESTING.md](TESTING.md) — Test framework, LogConfig debug-logging presets, suite files
- [COMPILER.md](COMPILER.md) — Lexer, parser, two-pass design, IR, type system, scope analysis, bytecode
- [DEVELOPMENT.md](DEVELOPMENT.md) — Contribution workflow and validation against CODE.md
