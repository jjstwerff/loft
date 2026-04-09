
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Developer Experience — 0.8.4 Designs

Designs for the DX items on the [ROADMAP](ROADMAP.md).

---

## SH.1 — TextMate Grammar

A `.tmLanguage.json` file that provides syntax highlighting for `.loft` files
in VS Code, Sublime Text, GitHub, and any editor that supports TextMate grammars.

### Scope mapping

| Loft construct | TextMate scope |
|---|---|
| `fn`, `struct`, `enum`, `type`, `pub`, `use`, `interface` | `keyword.declaration` |
| `if`, `else`, `for`, `while`, `match`, `in`, `return`, `break`, `continue`, `yield` | `keyword.control` |
| `and`, `or`, `as` | `keyword.operator` |
| `true`, `false` | `constant.language.boolean` |
| `null` | `constant.language.null` |
| `assert`, `debug_assert`, `panic`, `sizeof` | `keyword.other` |
| `integer`, `boolean`, `float`, `single`, `long`, `character`, `text` | `support.type` |
| `vector`, `sorted`, `hash`, `index` | `support.type.collection` |
| `not null` (two-word modifier) | `storage.modifier` |
| `CamelCase` identifiers | `entity.name.type` |
| `lower_case` after `fn ` | `entity.name.function` |
| `0x`, `0b`, `0o` prefixed numbers | `constant.numeric` |
| Decimal integers and floats | `constant.numeric` |
| `"..."` strings | `string.quoted.double` |
| `{expr}` inside strings | `meta.interpolation` / `punctuation.section.interpolation` |
| `{{` / `}}` inside strings | `constant.character.escape` |
| `\n`, `\t`, `\\`, `\"` | `constant.character.escape` |
| `//` to end of line | `comment.line.double-slash` |
| `/// ` doc comments | `comment.line.documentation` |
| `#rust`, `#native`, `#opcode` | `meta.annotation` |
| `@EXPECT_ERROR`, `@EXPECT_WARNING` | `meta.annotation.test` |

### String interpolation

Loft strings use `{expr}` for interpolation and `{{`/`}}` for literal braces.
The grammar must handle nested scopes inside `{...}`:

```json
{
  "begin": "\"",
  "end": "\"",
  "name": "string.quoted.double.loft",
  "patterns": [
    { "match": "\\\\[nrt\\\\\"0]", "name": "constant.character.escape.loft" },
    { "match": "\\{\\{|\\}\\}", "name": "constant.character.escape.loft" },
    {
      "begin": "\\{",
      "end": "\\}",
      "name": "meta.interpolation.loft",
      "patterns": [{ "include": "#expression" }]
    }
  ]
}
```

### Naming conventions

Loft enforces naming at the parser level:
- `CamelCase` = type/enum/variant names → scope as `entity.name.type`
- `lower_case` = variable/function names → default scope
- `UPPER_CASE` = constants → `constant.other`

The grammar can use regex `[A-Z][A-Za-z0-9]*` to detect CamelCase identifiers.

### File location

```
syntaxes/loft.tmLanguage.json
```

### Test

Open any `.loft` file in VS Code with the grammar installed; keywords, strings,
comments, types, and interpolation should all be coloured correctly.

---

## SH.2 — VS Code Extension

A minimal VS Code extension that bundles the TextMate grammar and provides
a good out-of-box experience for `.loft` files.

### Package structure

```
editors/vscode/
  package.json          — extension manifest
  syntaxes/
    loft.tmLanguage.json  — from SH.1
  language-configuration.json  — bracket matching, comment toggling, auto-closing
  snippets/
    loft.json           — fn, struct, enum, for, match snippets
  README.md             — marketplace description
```

### language-configuration.json

```json
{
  "comments": { "lineComment": "//" },
  "brackets": [
    ["{", "}"],
    ["[", "]"],
    ["(", ")"]
  ],
  "autoClosingPairs": [
    { "open": "{", "close": "}" },
    { "open": "[", "close": "]" },
    { "open": "(", "close": ")" },
    { "open": "\"", "close": "\"" }
  ],
  "surroundingPairs": [
    { "open": "{", "close": "}" },
    { "open": "[", "close": "]" },
    { "open": "(", "close": ")" },
    { "open": "\"", "close": "\"" }
  ],
  "indentationRules": {
    "increaseIndentPattern": "^.*\\{\\s*$",
    "decreaseIndentPattern": "^\\s*\\}"
  }
}
```

### Snippets (loft.json)

| Prefix | Expands to |
|---|---|
| `fn` | `fn name(params) -> type {\n\t$0\n}` |
| `struct` | `struct Name {\n\tfield: type,\n}` |
| `enum` | `enum Name {\n\tVariant,\n}` |
| `for` | `for item in collection {\n\t$0\n}` |
| `match` | `match expr {\n\t_ => $0,\n}` |
| `if` | `if condition {\n\t$0\n}` |

### Task definition

Add a `.vscode/tasks.json` template that lets users press Ctrl+Shift+B to run:
```json
{ "label": "Run loft", "command": "loft", "args": ["${file}"], "type": "shell" }
```

### Publishing

Publish to VS Code Marketplace as `loft-lang.loft` (or `jjstwerff.loft`).
Requires a Personal Access Token from https://dev.azure.com.

---

## DX.1 — Quick-Start Examples

An `examples/` directory at the repository root with self-contained programs
users can run immediately after install.

### Files

| File | Purpose | Demonstrates |
|---|---|---|
| `hello.loft` | Hello world | `println`, string interpolation |
| `fibonacci.loft` | Recursive + iterative fibonacci | Functions, loops, recursion |
| `fizzbuzz.loft` | Classic FizzBuzz | If/else, modulo, format strings |
| `structs.loft` | Point, distance calculation | Structs, methods, math |
| `collections.loft` | Vector, sorted, hash operations | Collection types, iteration |
| `match.loft` | Pattern matching on enums | Enum, match, guards |
| `files.loft` | Read/write a text file | File I/O |

### Requirements

- Each file must be runnable standalone: `loft examples/hello.loft`
- No dependencies on lib/ packages
- Each file should be under 30 lines with comments explaining key concepts
- Output should be self-explanatory (no "test passed" — show meaningful results)

### README update

Add to README.md after the installation section:

```markdown
## Examples

```sh
loft examples/hello.loft        # Hello world
loft examples/fibonacci.loft    # Fibonacci sequence
loft examples/structs.loft      # Structs and methods
```
```

---

## DX.2 — CI: Package Tests + Native Tests

Expand `.github/workflows/ci.yml` to run the full test suite that `make ci`
runs locally.

### Current CI jobs

1. Format (`cargo fmt -- --check`)
2. Clippy (`cargo clippy --tests -- -D warnings`)
3. Test (`cargo test`)

### New jobs to add

| Job | Command | Runs on | Purpose |
|---|---|---|---|
| Package tests | `make test-packages` | ubuntu, macos | Verify lib/ packages work |
| Native tests | `make test-native` | ubuntu, macos | Verify `--native` path |

### Implementation

Add to `.github/workflows/ci.yml` after the existing Test job:

```yaml
  Package-tests:
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo build --release
      - run: make test-packages

  Native-tests:
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo build --release
      - run: make test-native
```

Windows is excluded from native tests because `rustc` invocation paths differ.

---

## DX.3 — Error Messages: Source Location + Suggestions

Improve error messages to show the source file, line, column, and the
offending source line with a caret pointing to the error position.

### Current format

```
Error: Unknown variable 'z'
```

### Target format

```
error[E001]: unknown variable 'z'
  --> examples/hello.loft:5:12
   |
 5 |     result = z + 1;
   |              ^ not found in this scope
   |
   = help: did you mean 'x'?
```

### Implementation

#### Phase 1: Source location in diagnostics

The `Diagnostics` struct currently stores only `Vec<String>` messages.
Extend to store structured entries:

```rust
struct DiagEntry {
    level: Level,
    message: String,
    file: Option<String>,
    line: Option<u32>,
    column: Option<u32>,
}
```

The parser already tracks position via `Lexer.position()` — thread it through
to `diagnostic!` calls.

#### Phase 2: Source line display

When printing a diagnostic, read the source file (already loaded in the parser)
and display the offending line with a caret:

```rust
fn format_diagnostic(entry: &DiagEntry, source: &str) -> String {
    // Extract line from source, print with line number and caret
}
```

#### Phase 3: "Did you mean?" suggestions

Already partially implemented — `objects.rs` has `known_var_or_type` that
detects unknown variables.  Extend with Levenshtein distance to suggest the
closest matching name:

```rust
fn suggest_similar(name: &str, candidates: &[&str]) -> Option<&str> {
    candidates.iter()
        .filter(|c| levenshtein(name, c) <= 2)
        .min_by_key(|c| levenshtein(name, c))
}
```

### Files to modify

| File | Change |
|---|---|
| `src/diagnostics.rs` | Structured `DiagEntry`, display formatting |
| `src/parser/*.rs` | Thread file/line/column into `diagnostic!` calls |
| `src/main.rs` | Format diagnostics with source context on exit |

---

## NT.1 — Native Codegen: Fix Remaining Test Failures

Make `--native` reliable for all test scripts so it can confidently remain
the default execution mode.

### Current state

- `native_mode = true` is already the default in `src/main.rs:1131`
- `make test-native` runs all `tests/docs/*.loft` files through `--native`
- Some scripts still fail due to unhandled IR patterns in `src/generation/`

### Approach

See [NATIVE.md](NATIVE.md) for the full dependency graph of remaining failures.
The key blockers (from NATIVE.md analysis):

1. **Type inference gaps** — generated Rust code has missing type annotations
2. **Null guard patterns** — null sentinel checks not emitted for all types
3. **Iterator codegen** — `for` loops over sorted/hash/index not fully wired
4. **Text lifetime** — generated code uses `&str` where `String` is needed
5. **Struct-enum dispatch** — polymorphic method calls in generated code

### Validation

After fixes, `make test-native` must pass all scripts that `cargo test` passes.
Add `make test-native` to CI (DX.2) to prevent regressions.

---

## See also

- [ROADMAP.md](ROADMAP.md) — milestone placement for these items
- [NATIVE.md](NATIVE.md) — full native codegen design and failure analysis
- [PACKAGES.md](PACKAGES.md) — package manager architecture
- [LOFT.md](LOFT.md) — language syntax reference (for grammar design)
