// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Loft Code Formatter

> **Status: shipped** — implemented in `src/formatter.rs` (T2-0, 2026-03-16).
> `loft --format file.loft` formats in-place; `loft --format-check file.loft` exits 1 if not canonical.

A canonical, opinionated formatter for `.loft` source files — similar in philosophy to `gofmt`.
One right way to format code; no configuration.

---

## Design Goals

- **Low cognitive burden**: the rules fit on one page; you never have to think about them
- **Idempotent**: running the formatter twice produces the same output as running it once
- **Comment-preserving**: line comments (`//`) survive formatting unchanged
- **No configuration**: there is one canonical loft style

---

## Invocation

```
loft --format file.loft          # format in-place (overwrites)
loft --format-check file.loft    # exit 1 if file differs from formatted output
```

Or as a pipe:
```
loft --format -             # read stdin, write stdout
```

A `LOFT_FORMAT=1` env var can redirect formatted output to stdout without overwriting
(useful for editor integrations following the existing `LOFT_IR`/`LOFT_DUMP` convention).

---

## Implementation Approach: Token-Stream Formatter

The formatter works on the **raw token stream** (not the IR), so it can preserve comments
and handle files that do not yet fully parse.  It is a single-pass state machine.

### New file: `src/formatter.rs`

```
pub struct Formatter {
    depth: usize,           // current brace depth
    prev: Token,            // previous non-comment token kind
    context: Vec<Ctx>,      // stack: Block | StructLit | ArgList | ArrayLit
    out: String,            // accumulated output
    pending_blank: bool,    // emit a blank line before next top-level item
}
```

### Changes to `src/lexer.rs`

Add **`Mode::Raw`**: like `Mode::Code` but:
- yields `LexItem::LineComment(String)` for `// ...` lines
- yields `LexItem::Whitespace` tokens carrying the original newline count between tokens
  (only the *count* matters, not exact spacing — the formatter discards original spacing)

This is additive — the parser continues to use `Mode::Code`.

### Changes to `src/main.rs`

Parse `--format` / `--format-check` / `--format -` flags before the normal execution path.
Call `formatter::format_file(path)` or `formatter::format_stdin()`.

---

## Formatting Rules

### Indentation

- 2 spaces per depth level (matches existing codebase convention)
- Depth increases after `{`, decreases before `}`

### Blank Lines

| Context | Rule |
|---|---|
| Between top-level items (`fn`, `struct`, `enum`, `type`, `use`, constant) | 1 blank line |
| Inside a function body | preserve at most 1 blank line |
| Inside struct/enum body | no blank lines |

### Braces `{ }`

Opening brace **always** on the same line as the header.
Closing brace **always** on its own line, at the enclosing indent.

```loft
fn foo(x: integer) -> integer {
  x + 1
}
```

**No inline blocks** — even a one-liner body is expanded:
```loft
// input:   if x > 0 { return x; }
// output:
if x > 0 {
  return x;
}
```

This is the single most important rule: it eliminates all "should I inline this?" decisions.

### Semicolons `;`

Semicolons terminate statements and are followed by a newline at the same depth.
The formatter does **not** insert missing semicolons (that remains a parse error).

### Commas `,`

| Context | Rule |
|---|---|
| Function parameter list `fn f(a, b)` | space after comma, stay on one line; wrap if line > 80 cols |
| Function call `f(a, b)` | space after comma, stay on one line; wrap if line > 80 cols |
| Struct literal `Point { x: 1, y: 2 }` | space after comma, stay on one line |
| Struct/enum definition body | comma + newline (each field on its own line) |
| Array literal `[1, 2, 3]` | space after comma, stay on one line; wrap if line > 80 cols |

Trailing commas in struct/enum definitions are **stripped** (the formatter enforces
"no trailing comma" as the canonical style, matching the existing test files).
Trailing commas in call/param/array contexts are also stripped.

The heuristic for "struct/enum definition body vs struct literal" is purely syntactic:
- In a `struct Name {` or `enum Name {` header → definition mode (multi-line)
- After `=`, after a type name in expression position → literal mode (single-line)

**Line-length wrapping for param/call/array lists (> 80 cols):** emit each element on
its own line at `depth+1`, with the closing `)` or `]` on a new line at `depth`.
This handles long function signatures without needing the full Wadler/Lindig algorithm.
```loft
// short — stays on one line:
fn add(a: integer, b: integer) -> integer {

// long — each param on its own line:
fn process(
  source: vector<Record>,
  target: vector<Record>,
  options: Options,
) -> Result {
```

### Spaces Around Tokens

| Token(s) | Rule |
|---|---|
| Binary operators `+ - * / % == != < > <= >= && \|\| & \| ^ << >> ?? as` | 1 space before and after |
| Assignment operators `= += -= *= /= %=` | 1 space before and after |
| `->` (return type) | 1 space before and after |
| `=>` | 1 space before and after |
| `:` in type annotation `field: type`, `param: type` | no space before, 1 space after |
| `::` (path separator) | no spaces |
| `.` (field/method access) | no spaces |
| `(` in call | no space before |
| `[` in index | no space before |
| `,` | no space before, 1 space after (except before newline) |
| `!` (unary not) | no space after |
| `-` (unary minus) | no space after |
| `#` (loop attribute) | no space before or after |
| `?` | no space before |

### Keywords with Trailing Space

`fn`, `pub`, `struct`, `enum`, `type`, `use`, `if`, `else`, `for`, `in`, `return`,
`break`, `continue`, `loop`, `as`, `and`, `or`, `not`

Exception: `else if` → 1 space between `else` and `if`.

### Comments

Line comments (`// ...`) are preserved verbatim.
A comment on its own line keeps its current indentation depth (re-indented to `depth`).
A trailing comment (end of a line with code) is separated from the code by 2 spaces.

Block comments (`/* ... */`) are not currently used in loft and are not handled.

---

## Context Stack

To decide inline-vs-multiline and spacing, the formatter maintains a stack of contexts:

```
enum Ctx {
    Block,          // inside { } of fn/if/for/loop body — multi-line
    StructDef,      // inside struct/enum declaration — multi-line, comma+newline
    ArgList,        // inside ( ) of fn call or declaration — single-line
    ArrayLit,       // inside [ ] — single-line unless overflow
    StructLit,      // inside { } of struct/enum constructor — single-line
    FormatExpr,     // inside { } of a format string — passthrough
}
```

Context is pushed on `{`, `(`, `[` and popped on the matching closer.
Determining the context at `{`:
- Stack top is `Block` or `ArgList` and prev token is `)` or `->` type → `Block`
- Prev token is identifier or type keyword → `StructLit`
- After `struct`/`enum` keyword path → `StructDef`
- Inside a `CString` format expression → `FormatExpr`

---

## Token Reconstruction

The formatter scans the token stream and, for each token, emits:

1. Any pending newlines / indentation
2. Any spacing before the token (based on rules above)
3. The token text
4. Any comment that was attached to this line

The token → text mapping:

| LexItem | Output |
|---|---|
| `Integer(n, false)` | decimal `n` |
| `Integer(n, true)` | `0x{N:X}` for hex constants, else decimal |
| `Long(n)` | `{n}l` |
| `Float(f)` | shortest round-trip decimal |
| `Single(f)` | `{f}f` |
| `Token(s)` | `s` as-is |
| `Identifier(s)` | `s` as-is |
| `CString(s)` | `"{s}"` with interior `{...}` expressions re-formatted |
| `Character(c)` | `'c'` |
| `LineComment(s)` | `// {s}` |

---

## Example: Before and After

**Input (messy):**
```loft
fn   add( a:integer,b:integer)->integer{return a+b;}
struct  Point{x:float,y:float}
fn dist(p:Point)->float{ let d=p.x*p.x+p.y*p.y; d.sqrt() }
```

**Output (formatted):**
```loft
fn add(a: integer, b: integer) -> integer {
  return a + b;
}

struct Point {
  x: float,
  y: float
}

fn dist(p: Point) -> float {
  let d = p.x * p.x + p.y * p.y;
  d.sqrt()
}
```

---

## File Layout in `src/formatter.rs`

```
pub fn format_file(path: &str) -> Result<String, String>
  // read file, call format_source, return formatted string

pub fn format_source(source: &str, filename: &str) -> Result<String, String>
  // create Formatter, drive it with raw-mode lexer tokens

impl Formatter {
    fn next_token(&mut self, item: &LexItem)
    fn push_ctx(&mut self, ctx: Ctx)
    fn pop_ctx(&mut self) -> Ctx
    fn emit(&mut self, text: &str)
    fn emit_newline(&mut self)
    fn emit_space(&mut self)
    fn indent_str(&self) -> String
}
```

Total estimated size: ~400 lines of Rust.

---

## Non-Goals (Explicitly Out of Scope)

- **Full optimal line-length wrapping (Wadler/Lindig)**: the simple > 80-col heuristic on
  param/call/array lists covers the common cases; arbitrary expression wrapping is not attempted
- **Semantic analysis**: the formatter works on tokens alone; it does not type-check; files
  that do not yet compile can still be formatted
- **Multi-file project formatting**: `loft --format` takes one file at a time; shell
  globbing (`loft --format src/*.loft`) handles projects

## In Scope (revised from initial draft)

- **Import sorting**: consecutive `use` lines are sorted alphabetically within each contiguous
  block; a blank line between `use` statements starts a new block (preserving intentional grouping).
  This is safe because loft does not allow two `use` statements to import the same identifier — if
  they did, the second would already be a compile error.
- **Trailing comma stripping**: trailing commas after the last element in struct/enum definitions,
  call lists, and array literals are removed. Adding trailing commas is not done (requires
  knowing whether the grammar permits them in each position).

---

## Testing

Add `tests/format.rs`:
- `roundtrip_*` tests: format a known-good file, assert output == input (idempotent)
- `normalize_*` tests: format a deliberately messy input, assert it matches a golden file
- `comment_*` tests: verify comments survive at correct indentation
- `format_check_*` tests: verify exit code 0 for already-formatted, 1 for unformatted

Golden files go in `tests/format/` as `*.loft` (input) and `*.loft.fmt` (expected output).
