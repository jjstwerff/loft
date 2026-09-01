# Loft Language — VS Code Extension

Syntax highlighting and snippets for the [Loft programming
language](https://github.com/loft-lang/loft) in Visual Studio Code.

## Features

- **Run Loft File** button (▶ play icon) in the editor title bar.
  One click runs `loft <current-file>` in an integrated terminal.
  Default keybinding **F5**.
- **Run Loft File (Native)** button (🚀 rocket icon) next to it.
  Runs `loft --native <current-file>` for the AOT-compiled path.
  Default keybinding **Ctrl+F5** (Cmd+F5 on macOS).
- Both also appear in the right-click context menu (in the editor
  and the Explorer) and in the Command Palette.
- Syntax highlighting for `.loft` files (TextMate grammar — see
  `syntaxes/loft.tmLanguage.json`, symlinked to the canonical
  copy at the repo root)
- Comment toggling (`//`)
- Auto-closing for `{` `[` `(` `"` `` ` ``
- Auto-indent on `{` / `}`
- Snippets for common patterns: `fn`, `fnv`, `main`, `struct`,
  `enum`, `for`, `while`, `match`, `if`, `ife`, `assert`,
  `println`

A Language Server ships in this repository — build it with
`cargo build --release --bin loft-lsp`.  It answers diagnostics,
completion, go-to-definition, references, hover, document symbols,
semantic tokens, inlay hints, rename and formatting over stdio, and
`loft-dap` is the matching debug adapter.  What is still to do is
wiring it into THIS extension, which today provides highlighting,
snippets and the Run buttons — see
[`lib_plans/63-lsp/`](../../doc/claude/lib_plans/63-lsp/README.md).

## Configuration

Add to your VS Code settings (`Ctrl+,`):

```json
{
  "loft.binaryPath": "/path/to/your/loft"
}
```

Default is `loft`, which resolves via `$PATH`.  Set to an absolute
path if you have multiple loft installs and want this extension to
use a specific one (e.g. a local dev build at
`~/workspace/loft/target/release/loft`).

## Installation

### From VSIX (manual / pre-marketplace)

```sh
cd editors/vscode
npm install -g @vscode/vsce       # one-time, the package builder
npm install                       # one-time, fetches typescript + @types
vsce package                      # runs vscode:prepublish → tsc → bundles
code --install-extension loft-0.1.0.vsix
```

The first `npm install` fetches the TypeScript compiler and VS
Code type definitions used to build the run-button command
handlers in `src/extension.ts`.  Subsequent `vsce package` runs
re-trigger the build via the `vscode:prepublish` script.

### From the marketplace

Not yet published.  Tracked under ROADMAP 0.8.5 SH.2 publishing.

## Usage

Open any `.loft` file.  Keywords, types, strings (with `{interpolation}`),
numbers, and comments will be coloured according to your current theme.

Type a snippet prefix (e.g. `fn`, `for`, `match`) and press Tab to
expand.

### Run the current file

Add this to your workspace `.vscode/tasks.json` to run the active
`.loft` file with `Ctrl+Shift+B`:

```json
{
  "version": "2.0.0",
  "tasks": [
    {
      "label": "Run loft",
      "type": "shell",
      "command": "loft",
      "args": ["${file}"],
      "group": { "kind": "build", "isDefault": true },
      "presentation": { "reveal": "always" }
    }
  ]
}
```

The `loft` binary must be on your `PATH`.

## Source layout

```
editors/vscode/
├── package.json              ← extension manifest + contributes
├── tsconfig.json             ← TypeScript build config
├── language-configuration.json ← brackets, indent, auto-close
├── src/
│   └── extension.ts          ← Run / Run-Native command handlers
├── out/                      ← TypeScript compile output (gitignored)
│   └── extension.js
├── snippets/loft.json        ← snippet definitions
├── syntaxes/
│   └── loft.tmLanguage.json  → symlink to ../../../syntaxes/loft.tmLanguage.json
├── LICENSE                   → symlink to ../../LICENSE
├── .vscodeignore             ← deny-list for `vsce package`
└── README.md                 ← this file
```

The TextMate grammar lives canonically at the repo root
(`syntaxes/loft.tmLanguage.json`) so non-VS-Code editors can use
the same file.  The extension's `syntaxes/` directory is a
symlink; `vsce package` follows the symlink and bundles the
target file into the `.vsix`.

The `src/extension.ts` is the only TypeScript file — kept small
on purpose so non-frontend reviewers can audit it end-to-end.
It registers two commands (`loft.runFile`, `loft.runFileNative`)
that send `loft <file>` or `loft --native <file>` to a reusable
terminal named "Loft".

## Contributing

The grammar is hand-written JSON.  See
[`plans/36-developer-experience/` § SH.1](../../doc/claude/plans/36-developer-experience/README.md)
for the authoritative scope-name table; updates to either
should be mirrored.

## Licence

LGPL-3.0-or-later, matching the loft project.
