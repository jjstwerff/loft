# Loft Language — VS Code Extension

Syntax highlighting and snippets for the [Loft programming
language](https://github.com/jjstwerff/loft) in Visual Studio Code.

## Features

- Syntax highlighting for `.loft` files (TextMate grammar — see
  `syntaxes/loft.tmLanguage.json`, symlinked to the canonical
  copy at the repo root)
- Comment toggling (`//`)
- Auto-closing for `{` `[` `(` `"` `` ` ``
- Auto-indent on `{` / `}`
- Snippets for common patterns: `fn`, `fnv`, `main`, `struct`,
  `enum`, `for`, `while`, `match`, `if`, `ife`, `assert`,
  `println`

A full-featured Language Server (diagnostics, completion,
go-to-definition, hover) is planned for a future release —
see [LSP.md](../../doc/claude/LSP.md).

## Installation

### From VSIX (manual / pre-marketplace)

```sh
cd editors/vscode
npm install -g @vscode/vsce       # one-time
vsce package
code --install-extension loft-0.1.0.vsix
```

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
├── package.json              ← extension manifest
├── language-configuration.json ← brackets, indent, auto-close
├── snippets/loft.json        ← snippet definitions
├── syntaxes/
│   └── loft.tmLanguage.json  → symlink to ../../../syntaxes/loft.tmLanguage.json
└── README.md                 ← this file
```

The TextMate grammar lives canonically at the repo root
(`syntaxes/loft.tmLanguage.json`) so non-VS-Code editors can use
the same file.  The extension's `syntaxes/` directory is a
symlink; `vsce package` follows the symlink and bundles the
target file into the `.vsix`.

## Contributing

The grammar is hand-written JSON.  See
[`doc/claude/DX.md` § SH.1](../../doc/claude/DX.md) for the
authoritative scope-name table; updates to either should be
mirrored.

## Licence

LGPL-3.0-or-later, matching the loft project.
