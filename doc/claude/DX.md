
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

## DX.3 — Error Messages: Source Line Display + Suggestions *(completed)*

Errors already include `file:line:col` (e.g. `Error: Unknown variable 'zz'
at test.loft:1:31`).  This item adds source-line display with a caret and
"did you mean?" suggestions for unknown identifiers.

### Current output

```
Error: Unknown variable 'zz' at test.loft:1:31
```

### Target output

```
Error: Unknown variable 'zz' at test.loft:1:31
  |
1 |     y = x + zz;
  |             ^^ did you mean 'x'?
```

### Implementation

#### Phase 1: Structured diagnostic entries

The `Diagnostics` struct stores `Vec<String>`.  Change to structured entries
so the display layer can extract location info:

```rust
pub struct DiagEntry {
    pub level: Level,
    pub message: String,     // "Unknown variable 'zz'"
    pub file: String,        // "test.loft"
    pub line: u32,           // 1
    pub col: u32,            // 31
}
```

The `diagnostic!` macro already calls `self.lexer.diagnostic(level, msg)` which
formats the string with `position.file`, `position.line`, `position.pos`.
Change `Lexer::diagnostic` to push a `DiagEntry` instead of formatting a string.

#### Phase 2: Source line display in main.rs

`Parser` already holds the source text (loaded in `parse_file`).  Store a
`HashMap<String, String>` mapping file path → source content.  When printing
diagnostics in `main.rs`, look up the source, extract the line, and print
with a caret:

```rust
fn print_diagnostic(entry: &DiagEntry, sources: &HashMap<String, String>) {
    println!("{}: {} at {}:{}:{}", entry.level, entry.message,
             entry.file, entry.line, entry.col);
    if let Some(src) = sources.get(&entry.file) {
        if let Some(line_text) = src.lines().nth(entry.line as usize - 1) {
            let col = entry.col.saturating_sub(1) as usize;
            println!("  |");
            println!("{:>3} | {}", entry.line, line_text);
            println!("  | {:>width$}^", "", width = col + 1);
        }
    }
}
```

#### Phase 3: "Did you mean?" for unknown variables

When `known_var_or_type` in `objects.rs` reports an unknown variable, compute
Levenshtein distance against all in-scope variable names and suggest the
closest match (distance ≤ 2):

```rust
fn suggest_similar<'a>(name: &str, candidates: &[&'a str]) -> Option<&'a str> {
    candidates.iter()
        .copied()
        .filter(|c| levenshtein(name, c) <= 2)
        .min_by_key(|c| levenshtein(name, c))
}
```

Append ` — did you mean '{suggestion}'?` to the diagnostic message.
The Levenshtein function is ~15 lines of Rust (no external crate needed).

### Files to modify

| File | Change |
|---|---|
| `src/diagnostics.rs` | `DiagEntry` struct, `Diagnostics` stores `Vec<DiagEntry>` |
| `src/lexer.rs` | `Lexer::diagnostic` pushes `DiagEntry` instead of formatted string |
| `src/main.rs` | Source-line display when printing diagnostics |
| `src/parser/objects.rs` | Levenshtein suggestion on unknown variable |
| `src/parser/definitions.rs` | Levenshtein suggestion on unknown type |

---

## NT.1 — Native Codegen: Reliability *(completed)*

> **Status: all `make test-native` scripts pass (30/30 docs files).**
> Native mode is already the default (`src/main.rs:1131`).

The remaining work is regression prevention: add `make test-native` to CI
(DX.2) so native failures are caught before merge.  See [NATIVE.md](NATIVE.md)
for the full codegen design and any future N-series items.

---

## See also

- [ROADMAP.md](ROADMAP.md) — milestone placement for these items
- [NATIVE.md](NATIVE.md) — full native codegen design and failure analysis
- [PACKAGES.md](PACKAGES.md) — package manager architecture
- [LOFT.md](LOFT.md) — language syntax reference (for grammar design)
