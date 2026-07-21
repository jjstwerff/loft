<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# loft — Neovim integration

Wires the **`loft-lsp`** language server into Neovim's built-in LSP, so a `.loft`
buffer gets diagnostics, completion, hover, go-to-definition, find-references,
rename, code actions, inlay hints, semantic-token highlighting, document outline,
and tracker-tag (`@PLN63`) hovers — everything the language server ships. The
debug adapter (`loft-dap`) is auto-registered too, but only once it exists and
`nvim-dap` is installed (it's [not built yet](../../doc/claude/lib_plans/63-lsp/README.md#lsp3--loft-dap-debug-adapter-090));
until then this is LSP-only.

The LSP side is **dependency-free** — it uses `vim.lsp` (Neovim 0.8+), so you do
**not** need `nvim-lspconfig` or any plugin to try it.

## 1. Build the language server

```sh
cargo build --release --bin loft-lsp        # in the loft repo
```

Put it on your `$PATH` (or pass an absolute path in step 3):

```sh
export PATH="$PWD/target/release:$PATH"      # or copy target/release/loft-lsp into ~/.local/bin
```

Check it resolves: `command -v loft-lsp`.

## 2. Put this plugin on Neovim's runtimepath

Point Neovim at `editors/nvim` in your loft checkout. Any of:

- **Manual** (no plugin manager), in `init.lua`:
  ```lua
  vim.opt.runtimepath:append("/abs/path/to/loft/editors/nvim")
  ```
- **lazy.nvim**:
  ```lua
  { dir = "/abs/path/to/loft/editors/nvim", name = "loft", config = function() require("loft").setup() end }
  ```
- **Copy** `lua/loft.lua`, `ftdetect/loft.lua`, and `syntax/loft.vim` into the
  matching folders under `~/.config/nvim/`.

## 3. Enable it

In `init.lua`:

```lua
require("loft").setup()
-- or, if loft-lsp isn't on $PATH:
-- require("loft").setup({ loft_lsp = "/abs/path/to/loft/target/release/loft-lsp" })
```

That's it. Open any `.loft` file — the server attaches automatically (the root is
the nearest `loft.toml` or `.git`, else the cwd). `:LspInfo` / `:checkhealth lsp`
should show `loft-lsp` running.

## Keymaps (buffer-local, set on attach)

| Key | Action |
|-----|--------|
| `gd` | go to definition |
| `K` | hover (symbol signature + doc, or a tracker-tag card) |
| `grn` | rename (safe: refuses stdlib symbols; precise for locals / params) |
| `grr` | find references |
| `gra` | code action (e.g. "Change to `X`" quick-fixes) |
| `gO` | document symbols (outline) |
| `<leader>f` | format (runs `loft fmt`) |

**Completion** works through the LSP omni-completion: `<C-x><C-o>` in insert mode
(on Neovim 0.11+ it autotriggers). For popup-as-you-type, use a completion plugin
(`nvim-cmp`, `blink.cmp`, …) — the server is a standard LSP source. **Inlay hints**
are enabled automatically on Neovim 0.10+; toggle with
`vim.lsp.inlay_hint.enable()`.

## Options

`require("loft").setup({ … })` accepts:

- `loft_lsp` — absolute path to the `loft-lsp` binary (default: `loft-lsp` on `$PATH`).
- `loft_dap` — absolute path to `loft-dap` (default: `loft-dap` on `$PATH`; inert until the binary exists).

## Debugging (loft-dap) — future

Once `loft-dap` is built and `nvim-dap` is installed, `setup()` registers the
adapter and a "Run current file" configuration automatically — no extra config.
The scope of that adapter is the D0–D6 spine in
[the @PLN63 plan](../../doc/claude/lib_plans/63-lsp/README.md#lsp3--loft-dap-debug-adapter-090).

## Troubleshooting

- **No features / no attach** — `loft-lsp` isn't found. `command -v loft-lsp`; if
  empty, redo step 1–2 or pass `loft_lsp = "…"`.
- **Errors in `:messages`** — the server logs to stderr; run
  `loft-lsp` by hand against a file with `loft def <name>` / `loft symbols <file>`
  to sanity-check the binary independently of Neovim.
