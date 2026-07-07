
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# @PLN36 — Developer Experience

Designs for the DX items on the [ROADMAP](../../ROADMAP.md).
Originally drafted as "0.8.4 Designs"; several items slipped to
0.8.5 and beyond, so the version label was dropped.  Current
mix:

- **SH.1** TextMate grammar — **DONE** (shipped to
  `syntaxes/loft.tmLanguage.json`)
- **SH.2** VS Code extension — **DONE** (shipped to
  `editors/vscode/`)
- **SH.3** IntelliJ plugin — **DONE** (shipped to
  `editors/intellij/`; not in original spec but landed
  alongside SH.2)
- **DX.1** Quick-start `examples/` directory — **DONE**
  (7 example files at `examples/`: hello, fibonacci, fizzbuzz,
  structs, collections, match, files; README.md links them
  under § Three ways to see loft).
- **DX.3** "Learn loft in 30 minutes" walkthrough — **DONE**
  (`doc/learn-loft.md`).
- **DX.2** CI: package + native tests — **DONE** (verified
  2026-07-07). The goal — the full `make ci` suite in CI,
  native failures caught before merge — is satisfied by the
  evolved `ci.yml`: the per-PR `cargo nextest` run includes
  `binary(native)` (the `native` suite — `native_dir` sweeps
  `tests/docs/*.loft` under `--native` = `make test-native`;
  plus `native_library_suite`) and `binary(wrap)::library_suite`
  (builds/runs the `lib/*` native suites = the package tests),
  plus a dedicated **ASan UAF/OOB gate** (native) and nightly
  **`registry-validation.yml`** (every published package
  installed + native-built). Superseded the original 2-job
  spec (`make test-native` / `make test-packages`) with broader
  coverage; those exact jobs would only duplicate it.
- **NT.1** Native Codegen Reliability — completed (kept as
  historical design record)

**Plan status: ✅ COMPLETE (2026-07-07).** Originally drafted as
a six-item DX grab-bag; all six shipped/verified over the
0.8.4 / 0.8.5 cycle and beyond. The last item (DX.2) was
confirmed satisfied by the evolved CI (see above) — nothing
left to build. Issue [loft-lang/plans#36](https://github.com/loft-lang/plans/issues/36)
closed as `status:finished`; this dir stays in place as the
closure record.

Per-item landing procedures (build checklists, quality gates,
risks, decision points) live below in the **§ Landing
procedure (0.8.5 release)** section.

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
| `integer`, `boolean`, `float`, `single`, `character`, `text` | `support.type` |
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

## DX.2 — CI: Package Tests + Native Tests — ✅ DONE (verified 2026-07-07)

Goal met by the evolved `ci.yml` (see the plan Status block above) — the per-PR `cargo
nextest` run covers `binary(native)` + `binary(wrap)::library_suite`, plus the ASan gate and
nightly registry-validation. The original 2-job design below is kept as a historical record; it
was superseded, not implemented verbatim (those jobs would duplicate existing coverage).

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
(DX.2) so native failures are caught before merge.  See [NATIVE.md](../../NATIVE.md)
for the full codegen design and any future N-series items.

---

## Landing procedure (0.8.5 release)

Per-item ship criteria for the 0.8.5 release.  Generic per-
release content (tooling prerequisites, repository hygiene
gates, cross-platform smoke, release artefacts) lives in
[`../../../RELEASE.md`](../../RELEASE.md).

### SH.1 — TextMate grammar — ✅ DONE

**Effort:** ~45 min actual.
**Shipped:** `syntaxes/loft.tmLanguage.json` + `syntaxes/README.md`.

Build checklist (all done):
- Full SH.1 scope-mapping table per § SH.1 above.
- String interpolation `{expr}` as nested scope.
- `{{` / `}}` literal-brace escapes.
- Hex / binary / octal / decimal / float number variants.
- Doc comments (`///`) distinct from line comments (`//`).
- Test annotations (`@EXPECT_*`) distinct from compiler annotations.
- JSON validity verified via `python3 -m json.tool`.

Pre-ship visual sanity:
- Open `tests/docs/00-general.loft` / `06-function.loft` /
  `09-enum.loft` in VS Code with SH.2 installed; confirm
  keywords / strings / numbers / comments / interpolation all
  highlight correctly.
- Compare side-by-side against a Rust file (similar density).
- GitHub web UI: `*.loft linguist-language=Rust` in
  `.gitattributes` delivers ~80% via Rust's grammar; confirm
  acceptable on a sample loft file.

### SH.2 — VS Code extension scaffold — ✅ DONE

**Effort:** ~1 hour actual.
**Shipped:** `editors/vscode/` (package.json, snippets/loft.json,
language-configuration.json, syntaxes/loft.tmLanguage.json
symlinked to project root).

Build checklist (all done):
- `editors/vscode/package.json` (publisher / engine / activation
  / contributes.languages / contributes.grammars / contributes.snippets).
- `editors/vscode/language-configuration.json` (autoclose pairs,
  brackets, indentation, comment behaviour).
- `editors/vscode/snippets/loft.json` (10 essential snippets:
  fn, struct, enum, match, for, if/else, …).
- `editors/vscode/README.md` (install instructions + screenshot
  placeholder).
- Symlink to project root grammar (single source of truth).
- `vsce package` produces a clean `.vsix` (no warnings).

Pre-ship checks:
- Install the `.vsix` in fresh VS Code; open a `.loft` file;
  confirm syntax highlighting + snippet completion.
- Snippets trigger on the right prefixes.
- Comment toggle (Ctrl+/) wraps with `//` correctly.
- No "extension activation" errors in the VS Code Output panel.

### DX.1 — Quick-start `examples/` directory — ⬜ OPEN

**Effort:** XS (~1-2 hours).

Build checklist (per § DX.1 design above):
- `examples/hello.loft` — Hello world; `println` + interpolation.
- `examples/fibonacci.loft` — Recursive + iterative; functions /
  loops / recursion.
- `examples/fizzbuzz.loft` — If/else, modulo, format strings.
- `examples/structs.loft` — Point + distance; structs, methods, math.
- `examples/collections.loft` — Vector + sorted + hash; collection
  types, iteration.
- `examples/match.loft` — Pattern matching on enums.
- `examples/files.loft` — Read/write a text file; file I/O.

Per-file requirements:
- Each file runnable standalone: `loft examples/<name>.loft`.
- No dependencies on `lib/` packages.
- Each file < 30 lines.
- Comments explaining the key concept.
- Output self-explanatory.

Quality gates (before ship):
- All seven examples run cleanly under both `loft` and
  `loft --native`.
- Output review: read each example's output as a first-time user.
  Meaningful results, not "test passed".
- README.md update: append the "Examples" section per § DX.1.
- CI: add `examples/` to the test-suite shape used by
  `tests/scripts/`.  Catches regressions where stdlib changes
  break an example.

Risks:
- `files.loft` requires hermetic file I/O — write to `/tmp/` or
  example-created temp file.
- `collections.loft` may exceed 30 lines if it covers vector +
  sorted + hash all together.  Trim to one collection per file
  if so.

### DX.3 — "Learn loft in 30 minutes" walkthrough — ⬜ OPEN

**Effort:** S (~half a day).

Suggested structure (10 sections, ~3 minutes each):

| § | Title | Content |
|---|---|---|
| 1 | Hello, Loft | Install loft, run `loft examples/hello.loft`, edit it |
| 2 | Variables and types | No `let`; type inference; `assert` |
| 3 | Functions | `fn`, params, return type, default args, last-expr return |
| 4 | Control flow | `if`, `else`, `for`, `while`, `match` (one liners) |
| 5 | Strings | `"text"`, `{interpolation}`, `{{` escape, `len`, `+` |
| 6 | Collections | `vector`, `sorted`, `hash` — one example each |
| 7 | Structs | Define, construct, method-style call (`p.distance(q)`) |
| 8 | Enums | Plain enum + struct-enum variant; `match` on it |
| 9 | Files and JSON | Read a file, parse JSON, print a field |
| 10 | Where next | Pointer to STDLIB.md / this plan / `lib/` + GitHub issue tracker |

Build checklist:
- `doc/learn-loft.md` (or similar — pick a stable URL slug since
  README + extension marketplace will link to it).
- Each section ≤ 200 words of prose + 1-2 runnable code blocks.
- Every code block extracted from `examples/` (DX.1) so it's
  pre-verified to run.
- Closing section links to STDLIB.md, the examples directory,
  GitHub issue tracker.
- README.md links to the walkthrough in install / first-run section.

Quality gates (before ship):
- **Time check** — read it cold, top-to-bottom, no prior loft
  exposure.  25-35 minutes including running each code block.
  Trim a section if > 35.
- **External-reader check** — see § Ship-criterion test below.
- No broken examples — every code block runs as-is (CI extracts
  and runs them; or they're in `examples/` already and the
  walkthrough embeds them).
- Linker discipline — every term with a STDLIB.md / LOFT.md
  entry links to it on first use.

Risks:
- "30 minutes" is hand-wavy.  Acceptance = real external reader
  hits it.  Without that, the walkthrough is unverified
  marketing copy.

### Ship-criterion external test (per ROADMAP § 0.8.5)

The ROADMAP states:

> One external programmer (outside the loft project) can install
> SH.2 from VS Code Marketplace, open `examples/10-2d-canvas.loft`,
> read DX.3 top-to-bottom, and run the demo within 30 minutes from
> zero prior exposure.  Hands-on feedback collected before tagging.

Two known issues with this as written:

1. **SH.2 not on marketplace yet** — install from `.vsix` is the
   v1 path; marketplace publish gates on a separate Personal
   Access Token step (see Out of scope below).
2. **`examples/10-2d-canvas.loft` doesn't exist** — DX.1's table
   doesn't include it.  Either add it (would demonstrate 2D
   canvas / OpenGL bindings) or pick a different example for
   the ship criterion.

**Recommendation for 0.8.5:** swap to a small but visually
impressive example like `examples/hello.loft` + `examples/structs.loft`.
Add `examples/canvas.loft` only if the 2D canvas API surface is
stable enough to demo.

### Out of scope for 0.8.5 (tracked separately)

- **Marketplace publish of SH.2** — needs Personal Access Token
  + `vsce publish` invocation.  Separate publishing step.
- **GitHub Linguist first-class contribution** — submit PR to
  `github-linguist/linguist` adding loft.  Gated on Linguist's
  ~200-repo adoption rule.  Until then, the
  `*.loft linguist-language=Rust` directive in `.gitattributes`
  delivers ~80% via Rust's grammar.  Post-0.9.0 work.
- **NDB.1** — source-map-aware GDB / LLDB plugins.  See
  [`../25-native-debug/`](../34-native-debug), 0.8.6 work.
- **LSP server** — real LSP daemon.  See
  [`../../lib_plans/63-lsp/`](../../lib_plans/63-lsp),
  0.8.6 LSP.1.

### Decision points

1. **Should DX.1 add a `canvas.loft` example?**  Optional but
   it's what the ROADMAP ship criterion references.  If yes:
   ~1 hour extra.  If no: edit the ROADMAP ship criterion to
   pick a different example (`hello.loft` + `structs.loft` is
   the recommended swap above).
2. **Marketplace publish of SH.2 — block 0.8.5 or defer?**
   Recommend defer: `.vsix` install is the v1 path; marketplace
   publish lands as a 0.8.5.x patch.

---

## See also

- [ROADMAP.md](../../ROADMAP.md) — milestone placement for these items
- [NATIVE.md](../../NATIVE.md) — full native codegen design and failure analysis
- [PACKAGES.md](../../PACKAGES.md) — package manager architecture
- [LOFT.md](../../LOFT.md) — language syntax reference (for grammar design)
